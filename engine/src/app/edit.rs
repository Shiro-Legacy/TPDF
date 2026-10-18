// SPDX-License-Identifier: MIT
//! Object writes shared by the session's journaled `Transaction` and by a
//! document built from nothing (an extract, an image document), so the
//! graft, page-tree and form helpers serve both.
use std::sync::Arc;

use crate::document::{Document, DocumentError, DocumentErrorKind, DocumentLimits};
use crate::object::{Dictionary, Object, ObjectId};

use super::Transaction;
use super::document::{LowNumbers, MAX_OBJECT_NUMBER};

/// Write access to a document.
pub trait Edit {
    fn document(&self) -> &Document<'static>;
    /// Edits an object's value in place (a stream's dictionary, keeping its bytes).
    fn update<R>(
        &mut self,
        id: ObjectId,
        edit: impl FnOnce(&mut Object) -> R,
    ) -> Result<R, DocumentError>;
    /// Creates or replaces a non-stream object.
    fn put_value(&mut self, id: ObjectId, value: Object) -> Result<(), DocumentError>;
    /// Creates or replaces a stream whose `raw` bytes are encoded as its /Filter says.
    fn put_stream_raw(
        &mut self,
        id: ObjectId,
        dictionary: Dictionary,
        raw: Vec<u8>,
    ) -> Result<(), DocumentError>;
    /// A new non-stream object under a number never used before.
    fn allocate(&mut self, value: Object) -> Result<ObjectId, DocumentError>;
}

impl Edit for Transaction<'_> {
    fn document(&self) -> &Document<'static> {
        Transaction::document(self)
    }
    fn update<R>(
        &mut self,
        id: ObjectId,
        edit: impl FnOnce(&mut Object) -> R,
    ) -> Result<R, DocumentError> {
        Transaction::update(self, id, edit)
    }
    fn put_value(&mut self, id: ObjectId, value: Object) -> Result<(), DocumentError> {
        Transaction::put_value(self, id, value)
    }
    fn put_stream_raw(
        &mut self,
        id: ObjectId,
        dictionary: Dictionary,
        raw: Vec<u8>,
    ) -> Result<(), DocumentError> {
        Transaction::put_stream_raw(self, id, dictionary, raw)
    }
    fn allocate(&mut self, value: Object) -> Result<ObjectId, DocumentError> {
        Transaction::allocate(self, value, None)
    }
}

/// A document edited directly, with no journal: a new one (a catalog,
/// object 1, and an empty page tree, object 2) or one taken over, written
/// by the app writer once filled.
pub struct Builder {
    doc: Document<'static>,
    /// Numbers for new objects once the document's pass `MAX_OBJECT_NUMBER`.
    low: Option<LowNumbers>,
}

impl Builder {
    /// An empty `%PDF-<version>` document.
    pub fn new(version: &str) -> Result<Self, DocumentError> {
        let version = match version.as_bytes() {
            [major, b'.', minor] if major.is_ascii_digit() && minor.is_ascii_digit() => version,
            _ => "1.7",
        };
        let mut bytes = format!("%PDF-{version}\n").into_bytes();
        let catalog = bytes.len();
        bytes.extend_from_slice(b"1 0 obj\n<</Type/Catalog/Pages 2 0 R>>\nendobj\n");
        let pages = bytes.len();
        bytes.extend_from_slice(b"2 0 obj\n<</Type/Pages/Kids[]/Count 0>>\nendobj\n");
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!(
                "xref\n0 3\n0000000000 65535 f \n{catalog:010} 00000 n \n{pages:010} 00000 n \n\
                 trailer\n<</Size 3/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        let doc = Document::open_shared(Arc::new(bytes), DocumentLimits::default(), b"")?;
        Ok(Self { doc, low: None })
    }

    /// Edits an existing document with no journal (a file the session did
    /// not open, such as one a batch stamps).
    pub fn from_document(doc: Document<'static>) -> Self {
        Self { doc, low: None }
    }

    pub fn into_document(self) -> Document<'static> {
        self.doc
    }
}

impl Edit for Builder {
    fn document(&self) -> &Document<'static> {
        &self.doc
    }
    fn update<R>(
        &mut self,
        id: ObjectId,
        edit: impl FnOnce(&mut Object) -> R,
    ) -> Result<R, DocumentError> {
        self.doc.update_value(id, edit)
    }
    fn put_value(&mut self, id: ObjectId, value: Object) -> Result<(), DocumentError> {
        self.doc.put_value(id, value)
    }
    fn put_stream_raw(
        &mut self,
        id: ObjectId,
        dictionary: Dictionary,
        raw: Vec<u8>,
    ) -> Result<(), DocumentError> {
        self.doc.put_stream_raw(id, dictionary, raw)
    }
    /// Past the largest number while that is within `MAX_OBJECT_NUMBER`,
    /// else the lowest number no object holds or references (`LowNumbers`).
    fn allocate(&mut self, value: Object) -> Result<ObjectId, DocumentError> {
        let next = self.doc.next_id().map_or(u32::MAX, |id| id.number());
        if next <= MAX_OBJECT_NUMBER {
            return self.doc.allocate(value, None);
        }
        let low = (self.low).get_or_insert_with(|| LowNumbers::new(&self.doc, None));
        let id = (low.allocate())
            .and_then(|number| ObjectId::new(number, 0).ok())
            .ok_or(DocumentError {
                offset: 0,
                kind: DocumentErrorKind::LimitExceeded("object numbers"),
            })?;
        self.doc.put_value(id, value)?;
        Ok(id)
    }
}

/// A value inside an indirect object: the object, then dictionary keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub id: ObjectId,
    pub keys: Vec<Vec<u8>>,
}

impl Place {
    pub fn object(id: ObjectId) -> Self {
        Self {
            id,
            keys: Vec::new(),
        }
    }

    /// The value at this place, resolved; null when a key is missing.
    pub fn read<'d>(&self, doc: &'d Document<'_>) -> &'d Object {
        let mut value = doc.resolve_id(self.id);
        for key in &self.keys {
            value = match value.as_dict() {
                Some(dict) => doc.dict_get(dict, key),
                None => return &NULL,
            };
        }
        value
    }

    /// Where `key` of the dictionary at this place lives: its own object
    /// when the entry is a reference, otherwise inside this place.
    pub fn child(&self, doc: &Document<'_>, key: &[u8]) -> Place {
        let entry = self
            .read(doc)
            .as_dict()
            .and_then(|dict| dict.get(key))
            .map(|value| doc.resolve_with_holder(value));
        match entry {
            Some((Some(id), _)) => Place::object(id),
            _ => {
                let mut keys = self.keys.clone();
                keys.push(key.to_vec());
                Place { id: self.id, keys }
            }
        }
    }

    /// Runs `edit` on the value at this place; a missing (or null) key along
    /// the way is created as an empty dictionary. Fails when a step is not a
    /// dictionary.
    pub fn update<E: Edit, R>(
        &self,
        target: &mut E,
        edit: impl FnOnce(&mut Object) -> R,
    ) -> Result<R, DocumentError> {
        target
            .update(self.id, |value| {
                let mut value = value;
                for key in &self.keys {
                    let dict = value.as_dict_mut()?;
                    if matches!(dict.get(key.as_slice()), None | Some(Object::Null)) {
                        dict.insert(key.clone(), Object::Dictionary(Dictionary::new()));
                    }
                    value = dict.get_mut(key.as_slice())?;
                }
                Some(edit(value))
            })?
            .ok_or(DocumentError {
                offset: 0,
                kind: DocumentErrorKind::Invalid("edited value is not a dictionary"),
            })
    }
}

static NULL: Object = Object::Null;

/// The dictionary key `key` of a resolved dictionary value, or null.
pub fn get<'d>(doc: &'d Document<'_>, value: &'d Object, key: &[u8]) -> &'d Object {
    match doc.resolve(value).as_dict() {
        Some(dict) => doc.dict_get(dict, key),
        None => &NULL,
    }
}

/// Whether `value` resolves to the name `name`.
pub fn is_name(doc: &Document<'_>, value: &Object, name: &[u8]) -> bool {
    doc.resolve(value).as_name() == Some(name)
}
