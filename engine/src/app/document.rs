// SPDX-License-Identifier: AGPL-3.0-or-later
//! A session's open document: the live `Document`, its journal, and the
//! views derived from its content. All mutation goes through a
//! `Transaction`, which exists only while the journal records an operation,
//! so nothing changes the document without being journaled.
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use crate::document::{Document, DocumentError, DocumentErrorKind, DocumentLimits};
use crate::object::{Dictionary, Object, ObjectId};
use crate::repair::{RepairError, RepairLimits, rebuild};
use crate::view::{ViewDocument, ViewError};
use crate::writer::DocumentWriteError;

use super::info::{self, DocumentInfo, PageRef, PageState};
use super::journal::{Changed, Journal};
use super::overlay::{self, Base};

/// The parser limits of an app session: the app's input ceiling, no page cap.
pub fn limits() -> DocumentLimits {
    DocumentLimits {
        max_input_bytes: super::MAX_PDF_BYTES,
        max_xref_entries: 1_000_000,
        max_revisions: 512,
        max_metadata_decoded_bytes: 256 * 1024 * 1024,
        max_object_streams: 65_536,
        max_compressed_members: 1_000_000,
        max_objects: 8_000_000,
        max_decoded_bytes: 128 * 1024 * 1024,
        max_syntax_bytes: super::MAX_PDF_BYTES,
        ..DocumentLimits::default()
    }
}

/// Why bytes did not open.
#[derive(Debug)]
pub enum LoadError {
    Document(DocumentError),
    Repair(RepairError),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document(error) => error.fmt(f),
            Self::Repair(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for LoadError {}

/// The open document of a session.
pub struct AppDocument {
    doc: Document<'static>,
    /// The bytes `doc` was parsed from (rebuilt ones after a repair).
    input: Arc<Vec<u8>>,
    journal: Journal,
    /// The repair report called the input damaged (MuPDF's `wasRepaired`).
    repaired: bool,
    /// Opened without its password: structure only, never mutated or drawn.
    locked: bool,
    /// Bumped by every change of content, keying the view.
    state: u64,
    base: Option<Base>,
    /// Objects that may differ from `base`, and whether the trailer may.
    changed: BTreeSet<ObjectId>,
    trailer_changed: bool,
    view: Option<(u64, ViewDocument)>,
}

impl fmt::Debug for AppDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppDocument")
            .field("input", &self.input.len())
            .field("journal", &self.journal)
            .field("repaired", &self.repaired)
            .field("locked", &self.locked)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl AppDocument {
    /// Opens `input` as MuPDF would: strictly, else from a repair scan. When
    /// `password` does not authenticate, the document opens locked.
    pub fn load(input: Arc<Vec<u8>>, password: &[u8]) -> Result<Self, LoadError> {
        let strict = match Document::open_shared(input.clone(), limits(), password) {
            Ok(doc) => return Ok(Self::new(doc, input, false, false)),
            Err(error) => error,
        };
        match strict.kind {
            DocumentErrorKind::PasswordRequired => return Self::locked(input, false),
            DocumentErrorKind::LimitExceeded(_) => return Err(LoadError::Document(strict)),
            _ => {}
        }
        let repaired =
            rebuild(&input, RepairLimits::for_document(limits())).map_err(LoadError::Repair)?;
        let damaged = repaired.report.damaged;
        let bytes = Arc::new(repaired.bytes);
        match Document::open_shared(bytes.clone(), limits(), password) {
            Ok(doc) => Ok(Self::new(doc, bytes, damaged, false)),
            Err(e) if matches!(e.kind, DocumentErrorKind::PasswordRequired) => {
                Self::locked(bytes, damaged)
            }
            Err(e) => Err(LoadError::Document(e)),
        }
    }

    fn locked(input: Arc<Vec<u8>>, repaired: bool) -> Result<Self, LoadError> {
        let doc =
            Document::open_shared_locked(input.clone(), limits()).map_err(LoadError::Document)?;
        Ok(Self::new(doc, input, repaired, true))
    }

    fn new(doc: Document<'static>, input: Arc<Vec<u8>>, repaired: bool, locked: bool) -> Self {
        let base = (!locked)
            .then(|| Base::input(&doc, input.clone()))
            .flatten();
        Self {
            doc,
            input,
            journal: Journal::default(),
            repaired,
            locked,
            state: 0,
            base,
            changed: BTreeSet::new(),
            trailer_changed: false,
            view: None,
        }
    }

    pub fn document(&self) -> &Document<'static> {
        &self.doc
    }
    /// The bytes the document was parsed from.
    pub fn input(&self) -> &Arc<Vec<u8>> {
        &self.input
    }
    pub fn repaired(&self) -> bool {
        self.repaired
    }
    /// Opened without the password that it needs.
    pub fn is_locked(&self) -> bool {
        self.locked
    }
    pub fn journal(&self) -> &Journal {
        &self.journal
    }
    /// The journal position, as MuPDF numbers it.
    pub fn position(&self) -> i64 {
        i64::try_from(self.journal.position()).unwrap_or(i64::MAX)
    }
    pub fn can_undo(&self) -> bool {
        self.journal.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.journal.can_redo()
    }
    /// Journal step names, oldest first, the redo branch included.
    pub fn steps(&self) -> Vec<String> {
        self.journal.steps().map(str::to_string).collect()
    }
    /// Changes whenever the content does; equal values mean equal content.
    pub fn content_state(&self) -> u64 {
        self.state
    }

    /// engine.ts `detect(doc, needsPassword)`.
    pub fn info(&self, needs_password: bool) -> DocumentInfo {
        info::detect(&self.doc, needs_password, self.repaired)
    }
    pub fn page_state(&self) -> PageState {
        PageState::of(&self.doc)
    }
    pub fn page_count(&self) -> usize {
        info::pages(&self.doc).len()
    }
    /// The page list in display order (the view's order).
    pub fn pages(&self) -> Vec<PageRef<'_>> {
        info::pages(&self.doc)
    }
    /// The object holding each page's dictionary, in page order.
    pub fn page_ids(&self) -> Vec<Option<ObjectId>> {
        info::pages(&self.doc).iter().map(|page| page.id).collect()
    }
    /// Page `index`'s dictionary and the object holding it.
    pub fn page(&self, index: usize) -> Option<(Option<ObjectId>, &Dictionary)> {
        let page = *info::pages(&self.doc).get(index)?;
        Some((page.id, page.dict))
    }

    /// Starts recording an operation; false while one is open or when the
    /// document is locked.
    pub(crate) fn begin(&mut self, name: &str) -> bool {
        !self.locked && self.journal.begin(name)
    }
    /// Records the open operation (nothing when it touched nothing).
    pub(crate) fn end(&mut self) {
        let changed = self.journal.end(&self.doc);
        self.note(changed);
    }
    /// Restores everything the open operation touched.
    pub(crate) fn abandon(&mut self) -> Result<(), DocumentError> {
        let changed = self.journal.abandon(&mut self.doc)?;
        if !changed.is_empty() {
            self.state += 1;
        }
        self.note(changed);
        Ok(())
    }
    /// Reverts the last step and names it; None with nothing to undo.
    pub fn undo(&mut self) -> Result<Option<String>, DocumentError> {
        let moved = self.journal.undo(&mut self.doc)?;
        Ok(moved.map(|(name, changed)| self.moved(name, changed)))
    }
    /// Reapplies the next step and names it; None with nothing to redo.
    pub fn redo(&mut self) -> Result<Option<String>, DocumentError> {
        let moved = self.journal.redo(&mut self.doc)?;
        Ok(moved.map(|(name, changed)| self.moved(name, changed)))
    }
    fn moved(&mut self, name: String, changed: Changed) -> String {
        self.state += 1;
        self.note(changed);
        name
    }
    fn note(&mut self, changed: Changed) {
        self.changed.extend(changed.objects);
        self.trailer_changed |= changed.trailer;
    }

    /// Write access for the open operation; None outside one.
    pub fn transaction(&mut self) -> Option<Transaction<'_>> {
        (self.journal.is_open() && !self.locked).then_some(Transaction { app: self })
    }

    /// The current content drawn by hayro, cached until the content changes.
    pub fn view(&mut self) -> Result<&ViewDocument, ViewError> {
        if self.locked {
            return Err(ViewError::NeedsPassword);
        }
        if !matches!(&self.view, Some((state, _)) if *state == self.state) {
            // The old view's bytes go before the new ones are made.
            self.view = None;
            let bytes = self
                .render_bytes()
                .map_err(|_| ViewError::Invalid("current document bytes"))?;
            self.view = Some((self.state, ViewDocument::open(bytes, "")?));
        }
        match &self.view {
            Some((_, view)) => Ok(view),
            None => Err(ViewError::Invalid("current document bytes")),
        }
    }

    /// Objects and trailer that may differ from the base, including those
    /// the open operation has touched.
    fn overlay_set(&self) -> (BTreeSet<ObjectId>, bool) {
        let mut ids = self.changed.clone();
        ids.extend(self.journal.pending_objects());
        (ids, self.trailer_changed || self.journal.pending_trailer())
    }

    /// The current content as one plaintext PDF: the base plus an
    /// incremental update. Rebases when the update outgrows half the base.
    pub fn render_bytes(&mut self) -> Result<Vec<u8>, DocumentWriteError> {
        if self.locked {
            return Err(DocumentWriteError::Unsupported("locked document"));
        }
        if self.base.is_none() {
            self.rebase()?;
        }
        let (ids, trailer) = self.overlay_set();
        let Some(base) = &self.base else {
            return Err(DocumentWriteError::Invalid("render base"));
        };
        let mut update = Vec::new();
        let stale = base.stale(&self.doc, &ids);
        if !stale {
            update = overlay::overlay(&self.doc, base, &ids, trailer)?;
        }
        if stale || update.len() > (base.len() / 2).max(1 << 20) {
            self.rebase()?;
            update = Vec::new();
        }
        let Some(base) = &self.base else {
            return Err(DocumentWriteError::Invalid("render base"));
        };
        overlay::join(base, &update).map_err(DocumentWriteError::Serialize)
    }

    fn rebase(&mut self) -> Result<(), DocumentWriteError> {
        self.base = None;
        self.base = Some(Base::rewrite(&self.doc)?);
        self.changed.clear();
        self.trailer_changed = false;
        Ok(())
    }

    /// The bytes a reader of the live document should see: the input while
    /// nothing has changed in an encrypted document (it keeps its
    /// encryption), the render bytes otherwise. A locked document is its input.
    pub fn snapshot(&self) -> Result<Vec<u8>, DocumentWriteError> {
        if self.locked || (self.doc.security().is_some() && self.journal.position() == 0) {
            return Ok(self.input.to_vec());
        }
        let (ids, trailer) = self.overlay_set();
        match &self.base {
            Some(base) if !base.stale(&self.doc, &ids) => {
                let update = overlay::overlay(&self.doc, base, &ids, trailer)?;
                overlay::join(base, &update).map_err(DocumentWriteError::Serialize)
            }
            _ => Ok(Arc::unwrap_or_clone(Base::rewrite(&self.doc)?.bytes)),
        }
    }
}

/// Write access to the document during an operation. Every object and the
/// trailer are snapshotted on their first touch, so the journal can undo,
/// redo or abandon the operation exactly.
pub struct Transaction<'a> {
    app: &'a mut AppDocument,
}

impl Transaction<'_> {
    pub fn document(&self) -> &Document<'static> {
        &self.app.doc
    }

    fn write<R>(
        &mut self,
        id: ObjectId,
        change: impl FnOnce(&mut Document<'static>) -> Result<R, DocumentError>,
    ) -> Result<R, DocumentError> {
        let app = &mut *self.app;
        let first = app.journal.touch(&app.doc, id);
        let result = change(&mut app.doc);
        if result.is_err() && first {
            app.journal.untouch_if_unchanged(&app.doc, id);
        }
        app.state += 1;
        result
    }

    /// Creates or replaces a non-stream object.
    pub fn put_value(&mut self, id: ObjectId, value: Object) -> Result<(), DocumentError> {
        self.write(id, |doc| doc.put_value(id, value))
    }
    /// Replaces a non-stream object's value.
    pub fn replace_value(&mut self, id: ObjectId, value: Object) -> Result<(), DocumentError> {
        self.write(id, |doc| doc.replace_value(id, value))
    }
    /// Creates or replaces a stream whose `raw` bytes are encoded as the
    /// dictionary's /Filter says.
    pub fn put_stream_raw(
        &mut self,
        id: ObjectId,
        dictionary: Dictionary,
        raw: Vec<u8>,
    ) -> Result<(), DocumentError> {
        self.write(id, |doc| doc.put_stream_raw(id, dictionary, raw))
    }
    /// Creates or replaces a stream with unfiltered bytes.
    pub fn put_stream_decoded(
        &mut self,
        id: ObjectId,
        dictionary: Dictionary,
        decoded: Vec<u8>,
    ) -> Result<(), DocumentError> {
        self.write(id, |doc| doc.put_stream_decoded(id, dictionary, decoded))
    }
    /// Replaces an existing stream's bytes with unfiltered ones.
    pub fn replace_decoded_stream(
        &mut self,
        id: ObjectId,
        decoded: Vec<u8>,
    ) -> Result<(), DocumentError> {
        self.write(id, |doc| doc.replace_decoded_stream(id, decoded))
    }
    /// Replaces a stream's dictionary, keeping its bytes.
    pub fn set_stream_dict(
        &mut self,
        id: ObjectId,
        dictionary: Dictionary,
    ) -> Result<(), DocumentError> {
        self.write(id, |doc| doc.set_stream_dict(id, dictionary))
    }
    /// Edits an object's value in place (a stream's dictionary, keeping its
    /// bytes). A stream's value must stay a dictionary.
    pub fn update<R>(
        &mut self,
        id: ObjectId,
        edit: impl FnOnce(&mut Object) -> R,
    ) -> Result<R, DocumentError> {
        self.write(id, |doc| {
            let object = doc.object(id)?;
            let stream = object.stream.is_some();
            let mut value = object.value.clone();
            let result = edit(&mut value);
            match (stream, value) {
                (true, Object::Dictionary(dictionary)) => doc.set_stream_dict(id, dictionary)?,
                (true, _) => {
                    return Err(DocumentError {
                        offset: 0,
                        kind: DocumentErrorKind::Invalid("stream value must stay a dictionary"),
                    });
                }
                (false, value) => doc.put_value(id, value)?,
            }
            Ok(result)
        })
    }
    /// Removes an object and its stream bytes.
    pub fn remove_object(&mut self, id: ObjectId) -> Result<(), DocumentError> {
        self.write(id, |doc| doc.remove_object(id))
    }
    /// Removes a set of objects, refused when anything else refers to one.
    pub fn delete_objects(&mut self, ids: &BTreeSet<ObjectId>) -> Result<(), DocumentError> {
        let app = &mut *self.app;
        let first: Vec<ObjectId> = (ids.iter().copied())
            .filter(|&id| app.journal.touch(&app.doc, id))
            .collect();
        let result = app.doc.delete_objects(ids);
        if result.is_err() {
            for id in first {
                app.journal.untouch_if_unchanged(&app.doc, id);
            }
        }
        app.state += 1;
        result
    }
    /// A new object, one past the largest number; a stream gets unfiltered
    /// `bytes`.
    pub fn allocate(
        &mut self,
        value: Object,
        bytes: Option<Vec<u8>>,
    ) -> Result<ObjectId, DocumentError> {
        let id = self.app.doc.next_id()?;
        self.write(id, |doc| doc.allocate(value, bytes))
    }
    /// A new stream that keeps `raw` and its /Filter.
    pub fn allocate_stream_raw(
        &mut self,
        dictionary: Dictionary,
        raw: Vec<u8>,
    ) -> Result<ObjectId, DocumentError> {
        let id = self.app.doc.next_id()?;
        self.write(id, |doc| doc.allocate_stream_raw(dictionary, raw))
    }
    /// Sets (or with `None` removes) a trailer entry such as Info or ID.
    pub fn set_trailer_entry(
        &mut self,
        key: &[u8],
        value: Option<Object>,
    ) -> Result<(), DocumentError> {
        let app = &mut *self.app;
        let first = app.journal.touch_trailer(&app.doc);
        let result = app.doc.set_trailer_entry(key, value);
        if result.is_err() && first {
            app.journal.untouch_trailer();
        }
        app.state += 1;
        result
    }
}
