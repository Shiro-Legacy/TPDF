// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded transactions for the text-edit/save/reopen workflow.
//! Original source bytes are immutable. Parsing borrows a snapshot only within
//! an operation; history owns bytes and never holds self-referential borrows.
//! History keeps the original bytes, the current node's bytes and one object
//! delta per step; other nodes are replayed on undo/redo and checked against
//! the SHA-256 of the bytes they published. Limits bound retained PDF and
//! delta bytes, not total process RSS or allocator failure.
use crate::{
    content::{ContentLimits, ContentOperand, ContentParser},
    document::{Document, DocumentLimits},
    form_resources::OperationOrigin,
    object::{Object, ObjectId},
    page_glyph::{
        PageGlyphCommand, PageGlyphLimits, PageGlyphProgram, PageGlyphSession,
        PageGlyphSessionLimits, PageOutlinePolicy,
    },
    page_program::{PageMetricPolicy, PageProgram},
    page_raster::{
        PageRasterImage, PageRasterLimits, PageRasterSession, PageRasterSessionLimits, RasterTarget,
    },
    pages::{PageInfo, PageLimits, collect_pages},
    writer::{DocumentWriteLimits, rewrite_document, rewrite_reachable},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};
#[path = "session_features.rs"]
mod features;

static NEXT_SESSION: AtomicUsize = AtomicUsize::new(1);

#[derive(Clone, Copy, Debug)]
pub struct SessionLimits {
    pub max_pdf_bytes: usize,
    pub max_history_bytes: usize,
    pub max_history_steps: usize,
    pub max_pages: usize,
    pub max_text_bytes: usize,
    /// Edit, target-scan and render attempts; saving and history remain usable.
    pub max_operations: usize,
}
impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_pdf_bytes: 16 << 20,
            max_history_bytes: 64 << 20,
            max_history_steps: 64,
            max_pages: 64,
            max_text_bytes: 64 << 10,
            max_operations: 10_000,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionError {
    pub reason: &'static str,
    pub detail: String,
}
impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.reason, self.detail)
    }
}
impl std::error::Error for SessionError {}
fn err(reason: &'static str, detail: impl ToString) -> SessionError {
    SessionError {
        reason,
        detail: detail.to_string(),
    }
}
type Result<T> = std::result::Result<T, SessionError>;
type Dictionary = BTreeMap<Vec<u8>, Object>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalState {
    pub generation: usize,
    pub revision: u64,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}
/// Private fields prevent callers from forging a save receipt for another node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveToken {
    generation: usize,
    node: u64,
    sequence: u64,
}
pub struct SaveSnapshot {
    pub bytes: Vec<u8>,
    pub token: SaveToken,
}
#[derive(Clone, Debug)]
pub struct TextTarget {
    pub page: usize,
    pub text: String,
    pub bounds: [f64; 4],
    pub repertoire: String,
    generation: usize,
    revision: u64,
    operation: usize,
    stream: ObjectId,
    span: Range<usize>,
    font: usize,
    /// For text inside a Form placed by page content: the invocation index
    /// and the span of its `Do` in the joined page content.
    placement: Option<(usize, Range<usize>)>,
}
/// One object as changed by a history step, relative to the previous node.
enum Change {
    Removed(ObjectId),
    /// `bytes: None` on a stream keeps the previous node's raw bytes.
    Put {
        id: ObjectId,
        value: Object,
        stream: bool,
        bytes: Option<Vec<u8>>,
    },
}
struct Snapshot {
    node: u64,
    /// Changes from the previous node; empty for the original document.
    delta: Vec<Change>,
    /// Retained bytes charged for `delta`.
    delta_bytes: usize,
    /// SHA-256 of the exact bytes this node published, checked on replay.
    digest: [u8; 32],
    capabilities: SessionCapabilities,
}
/// An absent error means the bounded preview supports this operation. These
/// capabilities describe this snapshot, not general PDF specification support.
#[derive(Clone, Debug)]
pub struct PageCapabilities {
    pub render_error: Option<SessionError>,
    pub edit_error: Option<SessionError>,
    pub text_target_count: usize,
}
#[derive(Clone, Debug)]
pub struct SessionCapabilities {
    pub save_error: Option<SessionError>,
    pub pages: Vec<PageCapabilities>,
}
pub struct Session {
    limits: SessionLimits,
    /// The original document: node 0, and the base every delta replays onto.
    base: Vec<u8>,
    /// Bytes of the current node when it is not node 0.
    current: Option<Vec<u8>>,
    generation: usize,
    revision: u64,
    next_node: u64,
    history: Vec<Snapshot>,
    position: usize,
    saved_node: u64,
    save_sequence: u64,
    acknowledged_sequence: u64,
    attempts: usize,
    /// Held for reopening an encrypted snapshot; empty when none was given.
    password: Vec<u8>,
}
fn open_doc<'a>(bytes: &'a [u8], limits: SessionLimits, password: &[u8]) -> Result<Document<'a>> {
    Document::open_with_password(bytes, doc_limits(limits), password).map_err(|e| match e.kind {
        crate::document::DocumentErrorKind::PasswordRequired => err(
            "needs-password",
            "this document is encrypted; enter its password",
        ),
        _ => err("invalid-document", e),
    })
}
fn doc_limits(limits: SessionLimits) -> DocumentLimits {
    DocumentLimits {
        max_input_bytes: limits.max_pdf_bytes,
        ..DocumentLimits::default()
    }
}
fn write_limits(limits: SessionLimits) -> DocumentWriteLimits {
    let mut result = DocumentWriteLimits::default();
    result.syntax.max_output_bytes = limits.max_pdf_bytes;
    result
}
fn glyph_limits() -> PageGlyphSessionLimits {
    let mut result = PageGlyphSessionLimits::default();
    result.page.metric_policy = PageMetricPolicy::AllowPinnedBase14;
    result.outline_policy = PageOutlinePolicy::AllowPinnedSubstitutes;
    result
}
fn trace(doc: &Document<'_>, limits: SessionLimits, page: usize) -> Result<PageGlyphProgram> {
    let inventory = pages(doc, limits)?;
    let info = inventory
        .get(page)
        .ok_or_else(|| err("invalid-page", "page index"))?;
    trace_page_info(doc, info, page)
}
/// Page content followed by visible annotation appearances, as displayed.
fn trace_page_info(doc: &Document<'_>, info: &PageInfo, page: usize) -> Result<PageGlyphProgram> {
    let planned = features::page_annotations(doc, info, page)?;
    // Generated appearances live in a display-only copy of the document.
    let mut view = None;
    let mut annotations = Vec::with_capacity(planned.len());
    for plan in planned {
        annotations.push(match plan {
            features::PlannedAnnotation::Stored(appearance) => appearance,
            features::PlannedAnnotation::Generated {
                annotation,
                rect,
                appearance,
            } => {
                let view = view.get_or_insert_with(|| doc.clone());
                let mut dictionary = appearance.dictionary;
                if let Some(font) = appearance.font {
                    let font = view
                        .add_synthetic(Object::Dictionary(font), None)
                        .map_err(|e| err("too-large", e))?;
                    let fonts = [(
                        crate::generated_appearance::FONT_NAME.to_vec(),
                        Object::Reference(font),
                    )];
                    dictionary.insert(
                        b"Resources".to_vec(),
                        Object::Dictionary(
                            [(
                                b"Font".to_vec(),
                                Object::Dictionary(fonts.into_iter().collect()),
                            )]
                            .into_iter()
                            .collect(),
                        ),
                    );
                }
                let stream = view
                    .add_synthetic_stream(dictionary, appearance.content)
                    .map_err(|e| err("too-large", e))?;
                crate::page_program::AnnotationAppearance {
                    annotation: annotation.unwrap_or(stream),
                    stream,
                    rect,
                }
            }
        });
    }
    let doc = view.as_ref().unwrap_or(doc);
    PageGlyphSession::new(doc, glyph_limits())
        .map_err(|e| err("unsupported-page", e))?
        .trace_page_with_annotations(page, PageGlyphLimits::default(), &annotations)
        .map_err(|e| err("unsupported-page", e))
}
fn render_program(program: &PageGlyphProgram, dpi: f64) -> Result<PageRasterImage> {
    if !dpi.is_finite() || dpi <= 0.0 || dpi > 300.0 {
        return Err(err("invalid-scale", "DPI must be in (0, 300]"));
    }
    let geometry = program.page_program().geometry();
    let matrix = geometry
        .page_to_device(dpi)
        .map_err(|e| err("invalid-scale", e))?;
    let bounds = matrix
        .transform_bounds(geometry.bounds())
        .and_then(|r| r.outward_raster_bounds())
        .map_err(|e| err("too-large", e))?;
    let width = u32::try_from(bounds.width()).map_err(|e| err("too-large", e))?;
    let height = u32::try_from(bounds.height()).map_err(|e| err("too-large", e))?;
    if bounds.bounds()[0] != 0 || bounds.bounds()[1] != 0 {
        return Err(err("unsupported-page", "nonzero raster origin"));
    }
    PageRasterSession::new(PageRasterSessionLimits::default())
        .render(
            program,
            RasterTarget {
                width,
                height,
                page_to_device: matrix,
            },
            PageRasterLimits::default(),
        )
        .map_err(|e| err("unsupported-page", e))
}
fn pages(doc: &Document<'_>, limits: SessionLimits) -> Result<Vec<PageInfo>> {
    collect_pages(
        doc,
        PageLimits {
            max_pages: limits.max_pages,
            ..PageLimits::default()
        },
    )
    .map_err(|e| err("unsupported-document", e))
}
fn preflight(
    bytes: &[u8],
    limits: SessionLimits,
    strict: bool,
    password: &[u8],
) -> Result<SessionCapabilities> {
    let doc = open_doc(bytes, limits, password)?;
    // Whole-document write policy is checked before any edit is admitted.
    let save_error = if doc.security().is_some() {
        // Matches the application: encrypted documents cannot be changed.
        Some(err("encrypted", "encrypted documents open view-only"))
    } else {
        rewrite_document(&doc, write_limits(limits))
            .map_err(|e| err("unsupported-save", e))
            .err()
    };
    if strict {
        if let Some(error) = &save_error {
            return Err(error.clone());
        }
    }
    let inventory = pages(&doc, limits)?;
    let (document_error, restriction) = match features::check_document(&doc) {
        Ok(restriction) => (None, restriction),
        Err(error) => (Some(error), None),
    };
    if strict {
        if let Some(error) = document_error.as_ref().or(restriction.as_ref()) {
            return Err(error.clone());
        }
    }
    let save_error = save_error.or(restriction);
    if inventory.is_empty() {
        return Err(err("unsupported-document", "document has no pages"));
    }
    let mut capabilities = SessionCapabilities {
        save_error,
        pages: Vec::new(),
    };
    for (index, page) in inventory.iter().enumerate() {
        // A fresh bounded interpreter isolates a failed page from its neighbors.
        let program = (|| {
            if let Some(error) = &document_error {
                return Err(error.clone());
            }
            let program = trace_page_info(&doc, page, index)?;
            render_program(&program, 18.0)?;
            Ok(program)
        })();
        let render_error = program.as_ref().err().cloned();
        if strict {
            if let Some(error) = &render_error {
                return Err(error.clone());
            }
        }
        let mut text_target_count = 0;
        let edit_error = capabilities
            .save_error
            .clone()
            .or_else(|| render_error.clone())
            .or_else(|| {
                match targets(
                    &doc,
                    program.as_ref().expect("render succeeded"),
                    limits,
                    0,
                    0,
                ) {
                    Ok(items) => {
                        text_target_count = items.len();
                        None
                    }
                    Err(error) => Some(error),
                }
            });
        capabilities.pages.push(PageCapabilities {
            render_error,
            edit_error,
            text_target_count,
        });
    }
    // The preview's save validator requires complete rendering. Preserve that
    // contract while allowing viewing of the pages whose renderers succeeded.
    if capabilities.save_error.is_none() {
        if let Some(error) = capabilities
            .pages
            .iter()
            .find_map(|p| p.render_error.as_ref())
        {
            capabilities.save_error = Some(err(
                "unsupported-save",
                format!("Saving requires all pages to be supported: {error}"),
            ));
        }
    }
    if let Some(error) = &capabilities.save_error {
        for page in &mut capabilities.pages {
            page.edit_error = Some(error.clone());
            page.text_target_count = 0;
        }
    }
    Ok(capabilities)
}
impl Session {
    /// Original strict workflow admission, retained for regression and callers
    /// that require every page to render and the document to be writable.
    pub fn open(bytes: Vec<u8>, limits: SessionLimits) -> Result<Self> {
        Self::open_impl(bytes, limits, true, b"")
    }
    /// Admit a readable page tree even when individual operations are unavailable.
    /// Unsupported visible content is never returned as a successful page render.
    pub fn open_view(bytes: Vec<u8>, limits: SessionLimits) -> Result<Self> {
        Self::open_impl(bytes, limits, false, b"")
    }
    /// View an encrypted document; a wrong password reports `needs-password`.
    pub fn open_view_with_password(
        bytes: Vec<u8>,
        limits: SessionLimits,
        password: &[u8],
    ) -> Result<Self> {
        Self::open_impl(bytes, limits, false, password)
    }
    fn document(&self) -> Result<Document<'_>> {
        open_doc(self.bytes(), self.limits, &self.password)
    }
    fn open_impl(
        bytes: Vec<u8>,
        limits: SessionLimits,
        strict: bool,
        password: &[u8],
    ) -> Result<Self> {
        if bytes.len() > limits.max_history_bytes {
            return Err(err("too-large", "initial history"));
        }
        let capabilities = preflight(&bytes, limits, strict, password)?;
        let generation = NEXT_SESSION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| err("too-large", "session identity exhausted"))?;
        Ok(Self {
            limits,
            generation,
            revision: 0,
            next_node: 1,
            history: vec![Snapshot {
                node: 0,
                delta: Vec::new(),
                delta_bytes: 0,
                digest: Sha256::digest(&bytes).into(),
                capabilities,
            }],
            base: bytes,
            current: None,
            position: 0,
            saved_node: 0,
            save_sequence: 0,
            acknowledged_sequence: 0,
            attempts: 0,
            password: password.to_vec(),
        })
    }
    pub fn capabilities(&self) -> &SessionCapabilities {
        &self.history[self.position].capabilities
    }
    fn page_capabilities(&self, page: usize) -> Result<&PageCapabilities> {
        self.capabilities()
            .pages
            .get(page)
            .ok_or_else(|| err("invalid-page", "page index out of range"))
    }
    pub fn state(&self) -> JournalState {
        JournalState {
            generation: self.generation,
            revision: self.revision,
            dirty: self.history[self.position].node != self.saved_node,
            can_undo: self.position > 0,
            can_redo: self.position + 1 < self.history.len(),
        }
    }
    pub fn bytes(&self) -> &[u8] {
        self.current.as_deref().unwrap_or(&self.base)
    }
    /// PDF and delta bytes retained by history, as charged against
    /// `max_history_bytes`: the original, the current node and every delta.
    pub fn retained_history_bytes(&self) -> usize {
        self.history.iter().fold(
            self.base.len() + self.current.as_ref().map_or(0, Vec::len),
            |sum, node| sum.saturating_add(node.delta_bytes),
        )
    }
    pub fn pages(&self) -> Result<Vec<PageInfo>> {
        let doc = self.document()?;
        pages(&doc, self.limits)
    }
    fn charge(&mut self) -> Result<()> {
        self.attempts = self
            .attempts
            .checked_add(1)
            .ok_or_else(|| err("too-large", "operation count"))?;
        if self.attempts > self.limits.max_operations {
            return Err(err("too-large", "session operation budget"));
        }
        Ok(())
    }
    pub fn render(&mut self, page: usize, dpi: f64) -> Result<PageRasterImage> {
        self.charge()?;
        if let Some(error) = &self.page_capabilities(page)?.render_error {
            return Err(error.clone());
        }
        let doc = self.document()?;
        render_program(&trace(&doc, self.limits, page)?, dpi)
    }
    pub fn text_targets(&mut self, page: usize) -> Result<Vec<TextTarget>> {
        self.charge()?;
        if let Some(error) = &self.page_capabilities(page)?.edit_error {
            return Err(error.clone());
        }
        let doc = self.document()?;
        let program = trace(&doc, self.limits, page)?;
        targets(&doc, &program, self.limits, self.generation, self.revision)
    }
    pub fn replace_text(&mut self, target: &TextTarget, replacement: &str) -> Result<JournalState> {
        self.charge()?;
        if target.generation != self.generation || target.revision != self.revision {
            return Err(err(
                "stale-target",
                "rescan after editing, undo, redo or reopen",
            ));
        }
        if let Some(error) = &self.page_capabilities(target.page)?.edit_error {
            return Err(error.clone());
        }
        if replacement.len() > self.limits.max_text_bytes {
            return Err(err("too-large", "replacement text"));
        }
        let mut doc = self.document()?;
        let before = trace(&doc, self.limits, target.page)?;
        let eligible = targets(&doc, &before, self.limits, self.generation, self.revision)?;
        let actual = eligible
            .iter()
            .find(|t| {
                t.operation == target.operation
                    && t.stream == target.stream
                    && t.span == target.span
                    && t.placement == target.placement
            })
            .ok_or_else(|| err("stale-target", "target no longer eligible"))?;
        if replacement == actual.text {
            return Ok(self.state());
        }
        let mapping = repertoire(&doc, &before, actual.font);
        let mut encoded = Vec::new();
        for ch in replacement.chars() {
            encoded.push(*mapping.get(&ch).ok_or_else(|| {
                err(
                    "unsupported-encoding",
                    format!("character {ch:?} not proven drawable in this page font"),
                )
            })?);
        }
        let mut operator = Vec::with_capacity(encoded.len() * 2 + 5);
        operator.push(b'<');
        const HEX: &[u8] = b"0123456789ABCDEF";
        for byte in encoded {
            operator.push(HEX[(byte >> 4) as usize]);
            operator.push(HEX[(byte & 15) as usize]);
        }
        operator.extend_from_slice(b"> Tj");
        let splice = |content: &[u8], span: Range<usize>, with: &[u8]| -> Result<Vec<u8>> {
            let size = content
                .len()
                .checked_sub(span.len())
                .and_then(|n| n.checked_add(with.len()))
                .filter(|n| *n <= self.limits.max_pdf_bytes)
                .ok_or_else(|| err("too-large", "edited stream"))?;
            let mut changed = Vec::with_capacity(size);
            changed.extend_from_slice(&content[..span.start]);
            changed.extend_from_slice(with);
            changed.extend_from_slice(&content[span.end..]);
            Ok(changed)
        };
        let page_program = before.page_program();
        let page_content = page_program.content();
        let page_id = page_program.page_id();
        // Array joining padding is logical only and must never be serialized
        // into a member stream. The unchanged member boundaries stay intact.
        let page_segment = |span: &Range<usize>| {
            page_content
                .segments
                .iter()
                .find(|s| s.range.start <= span.start && s.range.end >= span.end)
                .ok_or_else(|| {
                    err(
                        "stale-target",
                        "operator no longer belongs to one content stream",
                    )
                })
        };
        match &actual.placement {
            None => {
                let segment = page_segment(&actual.span)?;
                let content = &page_content.bytes[segment.range.clone()];
                let span = (actual.span.start - segment.range.start)
                    ..(actual.span.end - segment.range.start);
                let changed = splice(content, span, &operator)?;
                write_page_stream(&mut doc, self.limits, page_id, segment.object, changed)?;
            }
            Some((invocation, call)) => {
                // Copy-on-write up the placement chain: the edited Form and
                // every Form placing it, up to the one the page places, are
                // copied. Each parent copy (and finally the page) names its
                // child copy under a fresh XObject name in its own resources.
                // Other placements, pages and the originals are unchanged.
                let invocations = page_program.form_invocations();
                let form_bytes = |placed: &crate::form_resources::FormInvocation| {
                    page_program.form_streams()[placed.stream_id().index()].bytes()
                };
                let mut placed = &invocations[*invocation];
                let mut edited = splice(form_bytes(placed), actual.span.clone(), &operator)?;
                let mut dictionary = dictionary_of(&doc, placed.object_id())?;
                loop {
                    // A Form drawn and named exactly once (typically an
                    // earlier copy) is edited in place; nothing above changes.
                    if private_form(&doc, page_program, placed) {
                        set_form(&mut doc, placed.object_id(), dictionary, edited)?;
                        break;
                    }
                    let copy = doc
                        .allocate(Object::Dictionary(dictionary), Some(edited))
                        .map_err(|e| err("too-large", e))?;
                    match placed.call_origin() {
                        OperationOrigin::PageContent { .. } => {
                            let segment = page_segment(call)?;
                            let content = &page_content.bytes[segment.range.clone()];
                            let call_span = (call.start - segment.range.start)
                                ..(call.end - segment.range.start);
                            let old = do_name(&content[call_span.clone()])?;
                            let name = add_page_xobject(&mut doc, page_id, copy, &old)?;
                            let changed = splice(content, call_span, &do_operator(&name))?;
                            write_page_stream(
                                &mut doc,
                                self.limits,
                                page_id,
                                segment.object,
                                changed,
                            )?;
                            break;
                        }
                        OperationOrigin::FormContent {
                            invocation: parent,
                            span,
                        } => {
                            let parent = &invocations[parent.index()];
                            let content = form_bytes(parent);
                            let old = do_name(&content[span.clone()])?;
                            dictionary = dictionary_of(&doc, parent.object_id())?;
                            let Some(resources) = dictionary.get(b"Resources".as_slice()) else {
                                return Err(err("stale-target", "Form resources changed"));
                            };
                            let Object::Dictionary(resources) = resolved(&doc, resources)? else {
                                return Err(err("invalid-edit", "Form Resources"));
                            };
                            let (resources, name) =
                                with_xobject(&doc, resources.clone(), copy, &old)?;
                            dictionary.insert(b"Resources".to_vec(), Object::Dictionary(resources));
                            edited = splice(content, span.clone(), &do_operator(&name))?;
                            placed = parent;
                        }
                        _ => return Err(err("stale-target", "Form placement changed")),
                    }
                }
            }
        }
        let candidate = rewrite_reachable(&doc, write_limits(self.limits))
            .map_err(|e| err("unsupported-save", e))?;
        let check = Document::open(&candidate, doc_limits(self.limits))
            .map_err(|e| err("invalid-save", e))?;
        let after = trace(&check, self.limits, target.page)?;
        validate_edit(&before, &after, actual.operation, replacement)?;
        render_program(&after, 18.0)?;
        self.commit(candidate)?;
        Ok(self.state())
    }
    fn commit(&mut self, bytes: Vec<u8>) -> Result<()> {
        // Capability results belong to exact history bytes. Recompute before
        // publication, so deletion of the last target cannot leave stale controls.
        let capabilities = preflight(&bytes, self.limits, false, b"")?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| err("too-large", "revision"))?;
        let next = self
            .next_node
            .checked_add(1)
            .ok_or_else(|| err("too-large", "history identity"))?;
        if self.position >= self.limits.max_history_steps {
            return Err(err("too-large", "history steps"));
        }
        let (delta, delta_bytes) = {
            let before = self.document()?;
            let after = Document::open(&bytes, doc_limits(self.limits))
                .map_err(|e| err("invalid-save", e))?;
            diff(&before, &after)?
        };
        // Redo nodes past the current position are discarded by this commit.
        self.history[1..=self.position]
            .iter()
            .try_fold(self.base.len(), |sum, node| {
                sum.checked_add(node.delta_bytes)
            })
            .and_then(|sum| sum.checked_add(bytes.len()))
            .and_then(|sum| sum.checked_add(delta_bytes))
            .filter(|n| *n <= self.limits.max_history_bytes)
            .ok_or_else(|| err("too-large", "history bytes"))?;
        self.history
            .try_reserve(1)
            .map_err(|e| err("too-large", e))?;
        self.history.truncate(self.position + 1);
        self.history.push(Snapshot {
            node: self.next_node,
            delta,
            delta_bytes,
            digest: Sha256::digest(&bytes).into(),
            capabilities,
        });
        self.current = Some(bytes);
        self.position += 1;
        self.next_node = next;
        self.revision = revision;
        Ok(())
    }
    pub fn undo(&mut self) -> Result<JournalState> {
        self.move_history(false)
    }
    pub fn redo(&mut self) -> Result<JournalState> {
        self.move_history(true)
    }
    fn move_history(&mut self, redo: bool) -> Result<JournalState> {
        if (redo && !self.state().can_redo) || (!redo && !self.state().can_undo) {
            return Err(err("no-history", "no operation to undo or redo"));
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| err("too-large", "revision"))?;
        let position = if redo {
            self.position + 1
        } else {
            self.position - 1
        };
        let bytes = self.materialize(position)?;
        self.current = bytes;
        self.position = position;
        self.revision = revision;
        Ok(self.state())
    }
    /// The exact bytes of history node `position` (None for the original),
    /// rebuilt by replaying deltas onto the original document.
    fn materialize(&self, position: usize) -> Result<Option<Vec<u8>>> {
        if position == 0 {
            return Ok(None);
        }
        let mut doc = open_doc(&self.base, self.limits, &self.password)?;
        for node in &self.history[1..=position] {
            for change in &node.delta {
                match change {
                    Change::Removed(id) => doc.remove(*id),
                    Change::Put {
                        id,
                        value,
                        stream,
                        bytes,
                    } => doc
                        .put(*id, value.clone(), *stream, bytes.clone())
                        .map_err(|e| err("invalid-history", e))?,
                }
            }
        }
        let bytes = rewrite_reachable(&doc, write_limits(self.limits))
            .map_err(|e| err("invalid-history", e))?;
        if Sha256::digest(&bytes).as_slice() != self.history[position].digest {
            return Err(err(
                "invalid-history",
                "replayed history differs from the committed snapshot",
            ));
        }
        Ok(Some(bytes))
    }
    pub fn save(&mut self) -> Result<SaveSnapshot> {
        if let Some(error) = &self.capabilities().save_error {
            return Err(error.clone());
        }
        let doc = self.document()?;
        let bytes = rewrite_reachable(&doc, write_limits(self.limits))
            .map_err(|e| err("unsupported-save", e))?;
        Document::open(&bytes, doc_limits(self.limits)).map_err(|e| err("invalid-save", e))?;
        self.save_sequence = self
            .save_sequence
            .checked_add(1)
            .ok_or_else(|| err("too-large", "save sequence"))?;
        Ok(SaveSnapshot {
            bytes,
            token: SaveToken {
                generation: self.generation,
                node: self.history[self.position].node,
                sequence: self.save_sequence,
            },
        })
    }
    /// Call only after the corresponding exact output has landed on disk.
    /// An edit made during the write stays dirty; older receipts cannot regress
    /// a newer acknowledged save. Receipts from other sessions are refused.
    pub fn acknowledge_saved(&mut self, token: SaveToken) -> Result<JournalState> {
        if token.generation != self.generation || token.sequence <= self.acknowledged_sequence {
            return Err(err(
                "stale-save",
                "save receipt belongs to another session or is superseded",
            ));
        }
        self.acknowledged_sequence = token.sequence;
        self.saved_node = token.node;
        Ok(self.state())
    }
}
/// Object-level changes from `before` to `after`, and the bytes they retain.
fn diff(before: &Document<'_>, after: &Document<'_>) -> Result<(Vec<Change>, usize)> {
    fn data<'d>(doc: &'d Document<'_>, id: ObjectId) -> Result<Option<&'d [u8]>> {
        doc.stream_data(id).map_err(|e| err("invalid-save", e))
    }
    let mut changes = Vec::new();
    let mut size = 0usize;
    for object in before.objects() {
        if after.object(object.id).is_err() {
            changes.push(Change::Removed(object.id));
            size = size.saturating_add(16);
        }
    }
    for object in after.objects() {
        let new = data(after, object.id)?;
        let (same_value, old) = match before.object(object.id) {
            Ok(old) => (old.value == object.value, data(before, object.id)?),
            Err(_) => (false, None),
        };
        if same_value && old == new {
            continue;
        }
        let bytes = new.filter(|_| old != new).map(<[u8]>::to_vec);
        size = size
            .saturating_add(value_size(&object.value))
            .saturating_add(bytes.as_ref().map_or(0, Vec::len));
        changes.push(Change::Put {
            id: object.id,
            value: object.value.clone(),
            stream: new.is_some(),
            bytes,
        });
    }
    Ok((changes, size))
}
/// Approximate retained size of a value: payload bytes plus a fixed per-node
/// overhead. Depth is capped at open, so the recursion is bounded.
fn value_size(value: &Object) -> usize {
    16 + match value {
        Object::Name(bytes) | Object::String(bytes) => bytes.len(),
        Object::Array(items) => items.iter().map(value_size).sum(),
        Object::Dictionary(items) => items.iter().map(|(k, v)| k.len() + value_size(v)).sum(),
        _ => 0,
    }
}
fn count_refs(value: &Object, counts: &mut BTreeMap<ObjectId, usize>) {
    match value {
        Object::Reference(id) => {
            if let Some(count) = counts.get_mut(id) {
                *count = count.saturating_add(1);
            }
        }
        Object::Array(items) => {
            for item in items {
                count_refs(item, counts);
            }
        }
        Object::Dictionary(items) => {
            for item in items.values() {
                count_refs(item, counts);
            }
        }
        _ => (),
    }
}
fn repertoire(doc: &Document<'_>, program: &PageGlyphProgram, font: usize) -> BTreeMap<char, u8> {
    let text = program.page_program().program().text();
    let mut candidates: BTreeMap<char, Option<u8>> = BTreeMap::new();
    for glyph in program.glyphs() {
        let code = &text.codes()[glyph.code_index()];
        if code.font_id().index() != font {
            continue;
        }
        let mut chars = text.text()[code.utf8_span()].chars();
        let Ok(byte) = u8::try_from(code.code()) else {
            continue;
        };
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            let slot = candidates.entry(ch).or_insert(Some(byte));
            if *slot != Some(byte) {
                *slot = None;
            }
        }
    }
    // Canonical fonts can prove additional drawable characters without relying
    // on their occurrence in the source page. Embedded fonts retain the existing
    // on-page repertoire until their full reverse-map gate is implemented.
    if let Some(info) = text
        .fonts()
        .iter()
        .find(|f| f.id().index() == font)
        .and_then(|f| {
            program
                .page_program()
                .fonts()
                .iter()
                .find(|info| info.name() == f.name())
        })
    {
        if let crate::font_resources::FontIdentity::Indirect(id) = info.identity() {
            let mut glyph_loader =
                crate::embedded_glyph::GlyphFontLoader::new(doc, Default::default());
            if let Ok(crate::embedded_glyph::LoadedGlyphFont::PinnedBase14(face)) =
                glyph_loader.load_source(id, PageOutlinePolicy::AllowPinnedBase14)
            {
                if let Ok(mut loader) =
                    crate::font_resources::FontResourceLoader::new(doc, Default::default())
                {
                    if let Ok(loaded) = loader.load(program.page_program().page_id(), info.name()) {
                        for code in 0u8..=255 {
                            let Some(name) = face.glyph_name(code) else {
                                continue;
                            };
                            if crate::builtin_outlines::BuiltinOutline::lookup(face.face(), name)
                                .is_none()
                            {
                                continue;
                            }
                            let input = [code];
                            if let Ok(decoded) = crate::font_text::decode_declared_text(
                                loaded,
                                &input,
                                Default::default(),
                            ) {
                                let mut chars = decoded.text().chars();
                                if let (Some(ch), None) = (chars.next(), chars.next()) {
                                    if ch.is_control() {
                                        continue;
                                    }
                                    candidates.entry(ch).or_insert(Some(code));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    candidates
        .into_iter()
        .filter_map(|(ch, code)| code.map(|c| (ch, c)))
        .collect()
}
fn targets(
    doc: &Document<'_>,
    program: &PageGlyphProgram,
    limits: SessionLimits,
    generation: usize,
    revision: u64,
) -> Result<Vec<TextTarget>> {
    features::check_text_edit(doc)?;
    let page = program.page_program();
    // Display substitution alone cannot prove portable saved replacement text.
    // Until embedding lands, retain the original embedded/Base14 edit policy.
    let mut edit_fonts = crate::embedded_glyph::GlyphFontLoader::new(doc, Default::default());
    for info in page.fonts() {
        if let crate::font_resources::FontIdentity::Indirect(id) = info.identity() {
            match edit_fonts.load_source(id, PageOutlinePolicy::AllowPinnedBase14) {
                Err(_) => {
                    return Err(err(
                        "not-candidate",
                        "This page uses a display substitute font. Portable text editing requires font embedding.",
                    ));
                }
                Ok(crate::embedded_glyph::LoadedGlyphFont::EmbeddedTrueType(font))
                    if font.cid_to_gid().is_some() =>
                {
                    return Err(err(
                        "not-candidate",
                        "This page uses a composite (CID) font. Editing two-byte text is not supported yet.",
                    ));
                }
                Ok(_) => {}
            }
        }
    }
    let content = page.content();
    let text = page.program().text();
    if text.codes().len() > 8192
        || content.bytes.len() > 1024 * 1024
        || content.segments.len() > 256
        || page.form_streams().len() > 256
    {
        return Err(err("too-large", "edit target scan budget"));
    }
    let info = pages(doc, limits)?
        .into_iter()
        .nth(page.page_index())
        .ok_or_else(|| err("invalid-page", "page index"))?;
    // Shared page streams are copied for this page when edited (M2), so they
    // may only be shared through other pages' /Contents, once per page.
    let uses = stream_uses(doc, limits, content.segments.iter().map(|s| s.object))?;
    for segment in &content.segments {
        let (references, contents) = uses[&segment.object];
        if references != contents
            || info
                .contents
                .iter()
                .filter(|id| **id == segment.object)
                .count()
                != 1
        {
            return Err(err("not-candidate", "shared stream requires copy-on-write"));
        }
    }
    let streams = std::iter::once(content.bytes.as_slice())
        .chain(page.form_streams().iter().map(|f| f.bytes()));
    for bytes in streams {
        let mut parser = ContentParser::new(bytes, ContentLimits::default());
        while let Some(op) = parser
            .next_operation()
            .map_err(|e| err("invalid-content", e))?
        {
            if matches!(op.operator.as_slice(), b"BDC" | b"DP") {
                return Err(err(
                    "not-candidate",
                    "Marked-content properties (ActualText, MCID) need structure-aware editing. This page can be viewed and saved unchanged.",
                ));
            }
        }
    }
    let mut by_operation: BTreeMap<usize, Vec<&crate::text_position::CodePlacement>> =
        BTreeMap::new();
    for code in text.codes() {
        by_operation
            .entry(code.operation_index())
            .or_default()
            .push(code);
    }
    let segment_of = |span: &Range<usize>| {
        content
            .segments
            .iter()
            .find(|s| s.range.start <= span.start && span.end <= s.range.end)
    };
    let mut result = Vec::new();
    for (&operation, codes) in &by_operation {
        if operation > 10_000 || result.len() > 256 {
            return Err(err("too-large", "edit target scan budget"));
        }
        // Operators may cross stream boundaries (even inside a string). Such
        // cases require an assembly transaction and are not local replacements.
        let (stream, span, bytes, placement) = match page.operation_origin(operation) {
            Some(OperationOrigin::PageContent { span }) => {
                let Some(segment) = segment_of(span) else {
                    continue;
                };
                (
                    segment.object,
                    span.clone(),
                    &content.bytes[span.clone()],
                    None,
                )
            }
            Some(OperationOrigin::FormContent { invocation, span }) => {
                let placed = &page.form_invocations()[invocation.index()];
                let Some(call) = placement_call(page, invocation.index()) else {
                    continue;
                };
                if segment_of(&call).is_none() {
                    continue;
                }
                let form = page.form_streams()[placed.stream_id().index()].bytes();
                (
                    placed.object_id(),
                    span.clone(),
                    &form[span.clone()],
                    Some((invocation.index(), call)),
                )
            }
            _ => continue,
        };
        let mut parser = ContentParser::new(bytes, ContentLimits::default());
        let Ok(Some(op)) = parser.next_operation() else {
            continue;
        };
        let show = (op.operator == b"Tj"
            && matches!(op.operands.as_slice(), [ContentOperand::String(_)]))
            || (op.operator == b"TJ"
                && matches!(op.operands.as_slice(), [ContentOperand::Array(_)]));
        let font = codes[0].font_id().index();
        if !show
            || !codes
                .iter()
                .all(|c| c.font_id().index() == font && c.rendering_mode() == 0)
        {
            continue;
        }
        let value: String = codes.iter().map(|c| &text.text()[c.utf8_span()]).collect();
        if value.len() > limits.max_text_bytes {
            continue;
        }
        let Some(bounds) = glyph_bounds(program, operation) else {
            continue;
        };
        let map = repertoire(doc, program, font);
        if value.chars().all(|ch| map.contains_key(&ch)) {
            result.push(TextTarget {
                page: page.page_index(),
                text: value,
                bounds,
                repertoire: map.keys().collect(),
                generation,
                revision,
                operation,
                stream,
                span,
                font,
                placement,
            });
        }
    }
    Ok(result)
}
/// The page-content span of the `Do` that starts this invocation's placement
/// chain, if every Form on it can be copied on write: the chain is not an
/// annotation appearance, and each Form placing another has its own
/// /Resources to receive the copy's fresh name.
fn placement_call(page: &PageProgram, invocation: usize) -> Option<Range<usize>> {
    let invocations = page.form_invocations();
    let mut placed = &invocations[invocation];
    // Each step moves to a strictly enclosing invocation.
    for _ in 0..invocations.len() {
        if placed.annotation().is_some() {
            return None;
        }
        match placed.call_origin() {
            OperationOrigin::PageContent { span } => return Some(span.clone()),
            OperationOrigin::FormContent { invocation, .. } => {
                placed = &invocations[invocation.index()];
                let scope = &page.resource_scopes()[placed.effective_scope().index()];
                if scope.provider() != Some(placed.object_id()) {
                    return None;
                }
            }
            _ => return None,
        }
    }
    None
}
/// Whether editing this Form object in place changes only this placement:
/// the page traces exactly one invocation of it (annotations included), and
/// its only reference in the document is in the object that places it, which
/// is this page itself or a parent Form that is private in the same sense.
/// Another page cannot resolve a page's own resources, so with one traced
/// invocation the Form is drawn exactly once in the whole document.
fn private_form<'p>(
    doc: &Document<'_>,
    page: &'p PageProgram,
    mut placed: &'p crate::form_resources::FormInvocation,
) -> bool {
    let invocations = page.form_invocations();
    for _ in 0..invocations.len() {
        let id = placed.object_id();
        if invocations.iter().filter(|i| i.object_id() == id).count() != 1 {
            return false;
        }
        let (holder, parent) = match placed.call_origin() {
            OperationOrigin::PageContent { .. } if placed.annotation().is_none() => {
                (page.page_id(), None)
            }
            OperationOrigin::FormContent { invocation, .. } => {
                let parent = &invocations[invocation.index()];
                (parent.object_id(), Some(parent))
            }
            _ => return false,
        };
        let mut holders = 0usize;
        for object in doc.objects() {
            let mut count = BTreeMap::from([(id, 0usize)]);
            count_refs(&object.value, &mut count);
            if count[&id] > 0 {
                if object.id != holder || count[&id] != 1 {
                    return false;
                }
                holders += 1;
            }
        }
        if holders != 1 {
            return false;
        }
        match parent {
            None => return true,
            Some(parent) => placed = parent,
        }
    }
    false
}
/// Replace a Form's dictionary and (unfiltered) content in place.
fn set_form(
    doc: &mut Document<'_>,
    id: ObjectId,
    mut dictionary: Dictionary,
    bytes: Vec<u8>,
) -> Result<()> {
    dictionary.remove(b"Filter".as_slice());
    dictionary.remove(b"DecodeParms".as_slice());
    let length = i64::try_from(bytes.len()).map_err(|_| err("too-large", "Form length"))?;
    dictionary.insert(b"Length".to_vec(), Object::Integer(length));
    doc.put(id, Object::Dictionary(dictionary), true, Some(bytes))
        .map_err(|e| err("invalid-edit", e))
}
fn do_name(call: &[u8]) -> Result<Vec<u8>> {
    let mut parser = ContentParser::new(call, ContentLimits::default());
    match parser.next_operation() {
        Ok(Some(op)) if op.operator == b"Do" => match op.operands.as_slice() {
            [ContentOperand::Name(name)] => Ok(name.clone()),
            _ => Err(err("stale-target", "Form placement changed")),
        },
        _ => Err(err("stale-target", "Form placement changed")),
    }
}
fn do_operator(name: &[u8]) -> Vec<u8> {
    [b"/".as_slice(), name, b" Do"].concat()
}
/// For each stream: (references from any object, uses as a page /Contents entry).
fn stream_uses(
    doc: &Document<'_>,
    limits: SessionLimits,
    ids: impl Iterator<Item = ObjectId>,
) -> Result<BTreeMap<ObjectId, (usize, usize)>> {
    let mut references: BTreeMap<_, _> = ids.map(|id| (id, 0)).collect();
    for object in doc.objects() {
        count_refs(&object.value, &mut references);
    }
    let mut uses: BTreeMap<_, _> = references.keys().map(|id| (*id, 0usize)).collect();
    for page in pages(doc, limits)? {
        for id in page.contents {
            if let Some(count) = uses.get_mut(&id) {
                *count = count.saturating_add(1);
            }
        }
    }
    Ok(references
        .into_iter()
        .map(|(id, r)| (id, (r, uses[&id])))
        .collect())
}
/// Replace one of `page`'s content streams. A stream other pages also use is
/// copied for this page, leaving every other use unchanged.
fn write_page_stream(
    doc: &mut Document<'_>,
    limits: SessionLimits,
    page: ObjectId,
    stream: ObjectId,
    bytes: Vec<u8>,
) -> Result<()> {
    let (_, uses) = stream_uses(doc, limits, std::iter::once(stream))?[&stream];
    if uses <= 1 {
        return doc
            .replace_decoded_stream(stream, bytes)
            .map_err(|e| err("invalid-edit", e));
    }
    let Object::Dictionary(dictionary) = doc
        .object(stream)
        .map_err(|e| err("invalid-edit", e))?
        .value
        .clone()
    else {
        return Err(err("invalid-edit", "content stream dictionary"));
    };
    let copy = doc
        .allocate(Object::Dictionary(dictionary), Some(bytes))
        .map_err(|e| err("too-large", e))?;
    let mut page_dict = dictionary_of(doc, page)?;
    let repoint = |value: &Object| match value {
        Object::Reference(id) if *id == stream => Object::Reference(copy),
        other => other.clone(),
    };
    let contents = match page_dict.get(b"Contents".as_slice()) {
        Some(Object::Reference(id)) if *id == stream => Object::Reference(copy),
        Some(Object::Reference(id)) => {
            match &doc.object(*id).map_err(|e| err("invalid-edit", e))?.value {
                Object::Array(items) => Object::Array(items.iter().map(repoint).collect()),
                _ => return Err(err("invalid-edit", "page Contents")),
            }
        }
        Some(Object::Array(items)) => Object::Array(items.iter().map(repoint).collect()),
        _ => return Err(err("invalid-edit", "page Contents")),
    };
    page_dict.insert(b"Contents".to_vec(), contents);
    doc.replace_value(page, Object::Dictionary(page_dict))
        .map_err(|e| err("invalid-edit", e))
}
fn dictionary_of(doc: &Document<'_>, id: ObjectId) -> Result<BTreeMap<Vec<u8>, Object>> {
    match &doc.object(id).map_err(|e| err("invalid-edit", e))?.value {
        Object::Dictionary(d) => Ok(d.clone()),
        _ => Err(err("invalid-edit", "page or Form dictionary")),
    }
}
fn resolved<'a>(doc: &'a Document<'_>, value: &'a Object) -> Result<&'a Object> {
    match value {
        Object::Reference(id) => Ok(&doc.object(*id).map_err(|e| err("invalid-edit", e))?.value),
        other => Ok(other),
    }
}
/// Give `page` its own copy of its effective resources whose /XObject maps a
/// fresh name to `form`, and return that name. Shared and inherited resource
/// dictionaries are left unchanged for every other user.
fn add_page_xobject(
    doc: &mut Document<'_>,
    page: ObjectId,
    form: ObjectId,
    base: &[u8],
) -> Result<Vec<u8>> {
    let mut current = page;
    let mut resources = BTreeMap::new();
    for _ in 0..64 {
        let dict = dictionary_of(doc, current)?;
        if let Some(value) = dict.get(b"Resources".as_slice()) {
            match resolved(doc, value)? {
                Object::Dictionary(d) => resources = d.clone(),
                _ => return Err(err("invalid-edit", "page Resources")),
            }
            break;
        }
        match dict.get(b"Parent".as_slice()) {
            Some(Object::Reference(parent)) => current = *parent,
            _ => break,
        }
    }
    let (resources, name) = with_xobject(doc, resources, form, base)?;
    let mut page_dict = dictionary_of(doc, page)?;
    page_dict.insert(b"Resources".to_vec(), Object::Dictionary(resources));
    doc.replace_value(page, Object::Dictionary(page_dict))
        .map_err(|e| err("invalid-edit", e))?;
    Ok(name)
}
/// `resources` (already a private copy) with its /XObject copied too and a
/// fresh name derived from `base` mapped to `form`. Shared or indirect
/// resource and XObject dictionaries are never modified.
fn with_xobject(
    doc: &Document<'_>,
    mut resources: Dictionary,
    form: ObjectId,
    base: &[u8],
) -> Result<(Dictionary, Vec<u8>)> {
    let mut xobjects = match resources.get(b"XObject".as_slice()) {
        Some(value) => match resolved(doc, value)? {
            Object::Dictionary(d) => d.clone(),
            _ => return Err(err("invalid-edit", "XObject resources")),
        },
        None => BTreeMap::new(),
    };
    let name = (1..=1000)
        .map(|n| [base, format!("_tpdf{n}").as_bytes()].concat())
        .find(|name| !xobjects.contains_key(name))
        .ok_or_else(|| err("too-large", "XObject names"))?;
    xobjects.insert(name.clone(), Object::Reference(form));
    resources.insert(b"XObject".to_vec(), Object::Dictionary(xobjects));
    Ok((resources, name))
}
fn glyph_bounds(program: &PageGlyphProgram, operation: usize) -> Option<[f64; 4]> {
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let text = program.page_program().program().text();
    for glyph in program.glyphs() {
        if text.codes()[glyph.code_index()].operation_index() != operation {
            continue;
        }
        for command in &program.commands()[glyph.command_range()] {
            let points = match *command {
                PageGlyphCommand::MoveTo(p) | PageGlyphCommand::LineTo(p) => vec![p],
                PageGlyphCommand::QuadTo { control, end } => vec![control, end],
                PageGlyphCommand::CubicTo {
                    control1,
                    control2,
                    end,
                } => vec![control1, control2, end],
                PageGlyphCommand::Close => vec![],
            };
            for point in points {
                bounds[0] = bounds[0].min(point.x());
                bounds[1] = bounds[1].min(point.y());
                bounds[2] = bounds[2].max(point.x());
                bounds[3] = bounds[3].max(point.y());
            }
        }
    }
    bounds.iter().all(|v| v.is_finite()).then_some(bounds)
}
fn validate_edit(
    before: &PageGlyphProgram,
    after: &PageGlyphProgram,
    operation: usize,
    replacement: &str,
) -> Result<()> {
    let new = after.page_program().program().text();
    let value: String = new
        .codes()
        .iter()
        .filter(|c| c.operation_index() == operation)
        .map(|c| &new.text()[c.utf8_span()])
        .collect();
    if value != replacement {
        return Err(err("unsupported-encoding", "replacement readback differs"));
    }
    // Fonts compare by identity: a copied Form has its own resource scope, so
    // registry indices may shift while the same font object draws the text.
    let unchanged = |program: &PageGlyphProgram| {
        let page = program.page_program();
        page.program()
            .text()
            .codes()
            .iter()
            .filter(|c| c.operation_index() != operation)
            .map(|c| {
                (
                    c.code(),
                    page.fonts().get(c.font_id().index()).map(|f| f.identity()),
                    c.transform().coefficients(),
                    c.rendering_mode(),
                )
            })
            .collect::<Vec<_>>()
    };
    if unchanged(before) != unchanged(after) {
        return Err(err("overflow", "replacement would move following text"));
    }
    if let Some(bounds) = glyph_bounds(after, operation) {
        let page = after.page_program().geometry().bounds().bounds();
        if bounds[0] < page[0] || bounds[1] < page[1] || bounds[2] > page[2] || bounds[3] > page[3]
        {
            return Err(err("overflow", "replacement extends outside page bounds"));
        }
    }
    Ok(())
}
