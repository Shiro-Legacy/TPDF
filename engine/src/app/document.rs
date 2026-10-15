// SPDX-License-Identifier: AGPL-3.0-or-later
//! A session's open document: the live `Document`, its journal, and the
//! views derived from its content. All mutation goes through a
//! `Transaction`, which exists only while the journal records an operation,
//! so nothing changes the document without being journaled.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use crate::document::{Document, DocumentError, DocumentErrorKind, DocumentLimits};
use crate::lexer::LexLimits;
use crate::object::{Dictionary, Object, ObjectId};
use crate::parser::ParseLimits;
use crate::repair::{RepairError, RepairLimits, rebuild};
use crate::view::{ViewDocument, ViewError};
use crate::writer::DocumentWriteError;

use super::SecuritySetting;
use super::info::{self, DocumentInfo, PageRef, PageState};
use super::journal::{Changed, Journal};
use super::layers::{Layer, LayerState};
use super::overlay::{self, Base, Removed};
use super::view_fixes;

/// MuPDF's `PDF_MAX_OBJECT_NUMBER`; a larger /Size does not set the
/// allocation floor.
const MAX_OBJECT_NUMBER: u32 = 8_388_607;

/// Limits a repair rebuild lifts by collapsing the file into one section.
const STRUCTURAL_LIMITS: [&str; 3] = ["xref revisions", "xref entries", "compressed members"];

/// The parser limits of an app session: the app's input ceiling, no page
/// cap, and a single object (a flat /Kids, a CIDFont /W) may use the whole
/// document budget. Objects are parsed eagerly (about 0.5 KiB of peak memory
/// each, measured), so xref entries stop at 2 million where MuPDF, which
/// loads lazily, takes 8 million.
pub fn limits() -> DocumentLimits {
    let max_objects = 8_000_000;
    let max_decoded_bytes = 128 * 1024 * 1024;
    DocumentLimits {
        max_input_bytes: super::MAX_PDF_BYTES,
        max_xref_entries: 2_000_000,
        max_revisions: 65_536,
        max_metadata_decoded_bytes: 256 * 1024 * 1024,
        max_object_streams: 65_536,
        max_compressed_members: 2_000_000,
        max_objects,
        max_decoded_bytes,
        max_syntax_bytes: super::MAX_PDF_BYTES,
        parse: ParseLimits {
            max_objects,
            max_decoded_bytes,
            // Deep structure trees parse (MuPDF reads 80 nested levels).
            max_depth: 256,
            lex: LexLimits {
                max_token_bytes: 32 * 1024 * 1024,
                max_tokens: 3 * max_objects,
            },
        },
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
    /// MuPDF's `countVersions` of the file as opened (see `versions`).
    versions: usize,
    /// Opened without its password: structure only, never mutated or drawn.
    locked: bool,
    /// Bumped by every change of content, keying the view.
    state: u64,
    /// No allocation takes a number below this: past every /Size, every
    /// object of the file and every number allocated before, so a stale or
    /// dangling reference never resolves to a new object. Undo and abandon
    /// leave it, as MuPDF's xref never shrinks.
    next_number: u32,
    /// What an acknowledged save wrote when it changed the source's security
    /// (`none` or `aes-256`); `None` while the file keeps the source's own.
    written_security: Option<SecuritySetting>,
    base: Option<Base>,
    /// Objects that may differ from `base`, and whether the trailer may.
    changed: BTreeSet<ObjectId>,
    trailer_changed: bool,
    /// The layers panel's view-only visibility; never journaled or saved.
    layers: LayerState,
    /// The view and the content state and layer serial it shows.
    view: Option<((u64, u64), ViewDocument)>,
    /// The edit ops' last page scan (engine.ts `editScanCache`).
    edit_scan: super::ops::edit::ScanCache,
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
        Self::load_any(input, &[password])
    }

    /// `load` with the first of `passwords` that authenticates (the
    /// encodings of one password, `password::candidates`). Junk before the
    /// header is skipped as MuPDF's bias skips it, without counting as
    /// damage. A size limit stops the open; the structural limits a rebuild
    /// collapses fall back to the repair scan.
    pub fn load_any(input: Arc<Vec<u8>>, passwords: &[&[u8]]) -> Result<Self, LoadError> {
        let mut failure = match open_any(&input, passwords) {
            Ok(Some(doc)) => return Ok(Self::new(doc, input, Opened::Strict)),
            Ok(None) => return Self::locked(input, Opened::Strict),
            Err(error) => error,
        };
        if let Some(shifted) = without_leading_junk(&input) {
            match open_any(&shifted, passwords) {
                Ok(Some(doc)) => return Ok(Self::new(doc, shifted, Opened::Strict)),
                Ok(None) => return Self::locked(shifted, Opened::Strict),
                Err(error) => failure = error,
            }
        }
        if matches!(failure.kind, DocumentErrorKind::LimitExceeded(reason)
            if !STRUCTURAL_LIMITS.contains(&reason))
        {
            return Err(LoadError::Document(failure));
        }
        let repaired =
            rebuild(&input, RepairLimits::for_document(limits())).map_err(LoadError::Repair)?;
        let report = &repaired.report;
        let opened = match report.damaged {
            true => Opened::Repaired,
            // A layout the strict reader refuses (hybrid, linearized): MuPDF
            // reads its sections as they are, less a linearized first page's.
            false => Opened::Rebuilt {
                versions: (report.xref_sections)
                    .saturating_sub(usize::from(report.linearized))
                    .max(1),
            },
        };
        let bytes = Arc::new(repaired.bytes);
        match open_any(&bytes, passwords) {
            Ok(Some(doc)) => Ok(Self::new(doc, bytes, opened)),
            Ok(None) => Self::locked(bytes, opened),
            Err(e) => Err(LoadError::Document(e)),
        }
    }

    fn locked(input: Arc<Vec<u8>>, opened: Opened) -> Result<Self, LoadError> {
        let doc =
            Document::open_shared_locked(input.clone(), limits()).map_err(LoadError::Document)?;
        Ok(Self::new_with(doc, input, opened, true))
    }

    fn new(doc: Document<'static>, input: Arc<Vec<u8>>, opened: Opened) -> Self {
        Self::new_with(doc, input, opened, false)
    }

    fn new_with(doc: Document<'static>, input: Arc<Vec<u8>>, opened: Opened, locked: bool) -> Self {
        let (repaired, versions) = match opened {
            Opened::Strict => (false, doc.trailer_chain().count()),
            // MuPDF rebuilds a damaged file into one section.
            Opened::Repaired => (true, 1),
            Opened::Rebuilt { versions } => (false, versions),
        };
        let base = (!locked)
            .then(|| Base::input(&doc, input.clone()))
            .flatten();
        let sizes =
            doc.trailer_chain()
                .filter_map(|trailer| match trailer.get(b"Size".as_slice()) {
                    Some(Object::Integer(size)) => u32::try_from(*size)
                        .ok()
                        .filter(|&size| size <= MAX_OBJECT_NUMBER + 1),
                    _ => None,
                });
        let next = doc.next_id().map_or(u32::MAX, |id| id.number());
        let next_number = sizes.chain([next]).max().unwrap_or(next);
        Self {
            doc,
            input,
            journal: Journal::default(),
            repaired,
            versions,
            locked,
            state: 0,
            next_number,
            written_security: None,
            base,
            changed: BTreeSet::new(),
            trailer_changed: false,
            layers: LayerState::default(),
            view: None,
            edit_scan: Default::default(),
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
    /// MuPDF's `countVersions` of the file as opened: its xref sections (a
    /// hybrid /XRefStm counted with its table), less a linearized file's
    /// first-page section, or 1 when it needed repair. Edits add none.
    pub fn versions(&self) -> usize {
        self.versions
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
    /// The security the last acknowledged save gave the file, when it was
    /// not the source's own: what `keep` keeps.
    pub fn written_security(&self) -> Option<&SecuritySetting> {
        self.written_security.as_ref()
    }
    /// Records an acknowledged save's `none` or `aes-256` setting; `keep`
    /// changes nothing.
    pub fn set_written_security(&mut self, setting: SecuritySetting) {
        if setting != SecuritySetting::Keep {
            self.written_security = Some(setting);
        }
    }
    /// Changes whenever the content does; equal values mean equal content.
    pub fn content_state(&self) -> u64 {
        self.state
    }

    /// engine.ts `detect(doc, needsPassword)`, its versions counted on the
    /// file as opened rather than on a rebuild of it.
    pub fn info(&self, needs_password: bool) -> DocumentInfo {
        DocumentInfo {
            versions: self.versions.saturating_sub(1),
            ..info::detect(&self.doc, needs_password, self.repaired)
        }
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
        let changed = self.journal.end();
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

    /// The optional-content groups with the layers panel's visibility.
    pub fn layers(&self) -> Vec<Layer> {
        self.layers.list(&self.doc)
    }
    /// Shows or hides a group in the view only (MuPDF `setLayerVisible`):
    /// no journal step, nothing saved.
    pub fn set_layer_visible(&mut self, ocg: ObjectId, visible: bool) {
        self.layers.set(&self.doc, ocg, visible);
    }

    /// The current content drawn by hayro with the layers panel's
    /// visibility and the view's own redefinitions (`view_fixes`), cached
    /// until either changes.
    pub fn view(&mut self) -> Result<&ViewDocument, ViewError> {
        if self.locked {
            return Err(ViewError::NeedsPassword);
        }
        let key = (self.state, self.layers.serial());
        if !matches!(&self.view, Some((shown, _)) if *shown == key) {
            // The old view's bytes go before the new ones are made.
            self.view = None;
            let unreadable = |_| ViewError::Invalid("current document bytes");
            let (mut fixes, mut streams) =
                view_fixes::redefinitions(&self.doc, self.layers.overrides());
            // Widget appearances are numbered after the annotations' and past
            // every number the journal may still hand out.
            let floor = self.allocation().map_or(u32::MAX, |id| id.number());
            let after =
                (streams.last_key_value()).map_or(0, |(id, _)| id.number().saturating_add(1));
            let widgets = super::forms::view::appearances(&self.doc, &mut fixes, floor.max(after));
            streams.extend(widgets);
            super::view_depth::shallow(&self.doc, &mut fixes);
            let bytes = (self.render_bytes_with(&fixes, &streams)).map_err(unreadable)?;
            self.view = Some((key, ViewDocument::open(bytes, "")?));
        }
        match &self.view {
            Some((_, view)) => Ok(view),
            None => Err(ViewError::Invalid("current document bytes")),
        }
    }

    /// The edit ops' scan cache.
    pub(crate) fn edit_scan(&mut self) -> &mut super::ops::edit::ScanCache {
        &mut self.edit_scan
    }

    /// `view` together with the document it shows, for ops that read both.
    pub fn view_with_document(&mut self) -> Result<(&ViewDocument, &Document<'static>), ViewError> {
        self.view()?;
        match &self.view {
            Some((_, view)) => Ok((view, &self.doc)),
            None => Err(ViewError::Invalid("current document bytes")),
        }
    }

    /// The id the next allocation takes: generation 0 of the allocation
    /// floor, or of one past the largest number in use if that is higher.
    fn allocation(&self) -> Result<ObjectId, DocumentError> {
        let number = self.doc.next_id()?.number().max(self.next_number);
        ObjectId::new(number, 0).map_err(|_| DocumentError {
            offset: 0,
            kind: DocumentErrorKind::LimitExceeded("object numbers"),
        })
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
        self.render_bytes_with(&BTreeMap::new(), &BTreeMap::new())
    }

    /// `render_bytes` followed by one more update redefining `objects` and
    /// adding `streams`.
    fn render_bytes_with(
        &mut self,
        objects: &BTreeMap<ObjectId, Object>,
        streams: &overlay::NewStreams,
    ) -> Result<Vec<u8>, DocumentWriteError> {
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
            update = overlay::overlay(&self.doc, base, &ids, trailer, Removed::Null)?;
        }
        if stale || update.len() > (base.len() / 2).max(1 << 20) {
            self.rebase()?;
            update = Vec::new();
        }
        let Some(base) = &self.base else {
            return Err(DocumentWriteError::Invalid("render base"));
        };
        overlay::join_redefined(base, &update, &self.doc, objects, streams)
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
                let update = overlay::overlay(&self.doc, base, &ids, trailer, Removed::Free)?;
                overlay::join(base, &[&update]).map_err(DocumentWriteError::Serialize)
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
    /// bytes), copying only what the edit reaches (`Document::update_value`).
    /// A stream's value must stay a dictionary.
    pub fn update<R>(
        &mut self,
        id: ObjectId,
        edit: impl FnOnce(&mut Object) -> R,
    ) -> Result<R, DocumentError> {
        self.write(id, |doc| doc.update_value(id, edit))
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
    /// A new object under a number never used before in this session (see
    /// `AppDocument::next_number`), generation 0. A stream (a dictionary
    /// with `bytes`) gets the unfiltered bytes.
    pub fn allocate(
        &mut self,
        value: Object,
        bytes: Option<Vec<u8>>,
    ) -> Result<ObjectId, DocumentError> {
        let id = self.app.allocation()?;
        match (value, bytes) {
            (value, None) => self.write(id, |doc| doc.put_value(id, value))?,
            (Object::Dictionary(dictionary), Some(bytes)) => {
                self.write(id, |doc| doc.put_stream_decoded(id, dictionary, bytes))?
            }
            (_, Some(_)) => {
                return Err(DocumentError {
                    offset: 0,
                    kind: DocumentErrorKind::Invalid("stream value must be a dictionary"),
                });
            }
        }
        self.allocated(id);
        Ok(id)
    }
    /// A new stream that keeps `raw` and its /Filter, numbered as `allocate`.
    pub fn allocate_stream_raw(
        &mut self,
        dictionary: Dictionary,
        raw: Vec<u8>,
    ) -> Result<ObjectId, DocumentError> {
        let id = self.app.allocation()?;
        self.write(id, |doc| doc.put_stream_raw(id, dictionary, raw))?;
        self.allocated(id);
        Ok(id)
    }
    fn allocated(&mut self, id: ObjectId) {
        self.app.next_number = id.number().saturating_add(1).max(self.app.next_number);
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

/// How `AppDocument::load_any` read its bytes.
#[derive(Clone, Copy)]
enum Opened {
    /// As they are (junk before the header skipped).
    Strict,
    /// Rebuilt from a repair scan of a damaged file.
    Repaired,
    /// Rebuilt from the file's own xref, whose layout the strict reader
    /// refuses; `versions` counts that xref as MuPDF does.
    Rebuilt { versions: usize },
}

/// The document `input` holds, opened with the first of `passwords` that
/// authenticates; `None` when it needs a password none of them is.
fn open_any(
    input: &Arc<Vec<u8>>,
    passwords: &[&[u8]],
) -> Result<Option<Document<'static>>, DocumentError> {
    let passwords = if passwords.is_empty() {
        &[b"".as_slice()][..]
    } else {
        passwords
    };
    for password in passwords {
        match Document::open_shared(input.clone(), limits(), password) {
            Ok(doc) => return Ok(Some(doc)),
            Err(error) if matches!(error.kind, DocumentErrorKind::PasswordRequired) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

/// `input` from its `%PDF-` header on, when junk precedes a header in the
/// first kilobyte (MuPDF's `pdf_load_version` reads offsets after a bias).
fn without_leading_junk(input: &[u8]) -> Option<Arc<Vec<u8>>> {
    let head = &input[..input.len().min(1024)];
    let at = head.windows(5).position(|window| window == b"%PDF-")?;
    let rest = input.get(at..).filter(|_| at > 0)?;
    Some(Arc::new(rest.to_vec()))
}
