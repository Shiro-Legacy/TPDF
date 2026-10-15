// SPDX-License-Identifier: AGPL-3.0-or-later
//! Document-bound, owned mixed page traces. This is not glyph or image rendering.
//!
//! Font caches have the independent `fonts` partition of the session budget.
//! Image metadata/decoding/cache use the independent `images` partition. Owned
//! image output handles, one possible boxed error, and frozen call records use the shared Form traversal
//! partition; each image Do has one source origin and no synthetic boundaries.
//! Content decoding, both parser passes and execution have cumulative ledgers.
//! Phase reservations are admitted before work and refunded only on success;
//! failed content/parser phases retain their conservative reservation. Parser
//! node/payload/token storage is a separate bounded partition, not output memory.
//! Session page inventory/geometry/ID storage is independently bounded by
//! `fonts.pages` and constructed once, outside the per-attempt output ledger.
//! `allocated_bytes` counts conservative full output/orchestration allocation
//! requests, including retained logical content; it excludes the borrowed open
//! document and the separately bounded font cache and parser temporaries.
//! Form metadata/source handles/origins/frames have the independent `forms`
//! partition. Form decoded buffers consume the shared PP1 encoded/decode/output
//! partition. `content.max_streams` bounds the root Contents list; actual Form
//! attempts are bounded by forms.max_invocations, and cumulative content_references
//! counts root references plus each attempted resolved Do. Parser limits apply to
//! each whole source pass (never each resumed suffix), under shared session totals.
//! Form source decoding reserves six times stage output plus 12288 bytes per
//! possible stage, including Flate's <=4096-byte initial spare capacity. Success
//! retains this conservative envelope for measured stage output; failure retains
//! the full admitted reservation. No-Do pages allocate no diagnostic buffer.
use crate::content::{
    ContentCheckpoint, ContentError, ContentLimits, ContentOperand, ContentParser, ContentUsage,
    Operation,
};
use crate::document::Document;
use crate::font_resources::{
    FontIdentity, FontResourceError, FontResourceErrorKind, FontResourceLimits, FontResourceLoader,
    FontResourceUsage, SimpleFontMapping,
};
pub use crate::form_resources::{
    FormDiagnosticFrame, FormError, FormInvocation, FormInvocationId, FormLimits, FormPhase,
    FormStream, FormStreamId, FormUsage, OperationOrigin, ResourceScope, ResourceScopeId,
};
use crate::form_resources::{FormErrorKind, FormResourceLoader, XObjectDefinition};
use crate::geometry::{Affine, Rect, UserSpace};
use crate::image_resources::{ImageAsset, ImageError, ImageLimits, ImageLoader, ImageUsage};
use crate::object::{Dictionary, Object, ObjectId};
use crate::operators::{OperatorError, OperatorKind, ValidatedOperation, validate_operation};
use crate::optional_content::OptionalContent;
use crate::page_content::{
    ContentSegment, PageContent, PageContentError, PageContentLimits,
    load_page_content_with_allocation,
};
use crate::page_geometry::{PageGeometry, PageGeometryError};
use crate::pages::{PageError, PageInfo, collect_pages};
use crate::path::InitialClip;
use crate::program::{
    FormFrame, Program, ProgramError, ProgramInitialState, ProgramInterpreter, ProgramLimits,
};
use crate::stream_object::{StreamObjectError, decode_document_stream_with_usage};
use crate::text_font::{
    TextFont, TextFontError, TextFontId, TextFontLimits, TextFonts, TextMetricKind,
    TextUnicodeKind, UnicodeSource,
};
use std::{fmt, mem::size_of, ops::Range, sync::Arc};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PageMetricPolicy {
    #[default]
    DeclaredOnly,
    AllowPinnedBase14,
}
#[derive(Clone, Copy, Debug)]
pub struct PageProgramSessionLimits {
    pub metric_policy: PageMetricPolicy,
    /// Independent nonoverlapping font/cache partition, retained across calls.
    pub fonts: FontResourceLimits,
    pub forms: FormLimits,
    /// Independent image decode/cache partition, retained across calls.
    pub images: ImageLimits,
    pub max_page_attempts: usize,
    pub max_content_references: usize,
    pub max_encoded_bytes: usize,
    pub max_decoded_work_bytes: usize,
    pub max_joined_bytes: usize,
    pub max_parser_input_bytes: usize,
    pub max_parser_nodes: usize,
    pub max_parser_owned_bytes: usize,
    pub max_parser_tokens: usize,
    pub max_parser_operations: usize,
    pub max_prepared_fonts: usize,
    pub max_font_name_bytes: usize,
    pub max_program_operations: usize,
    pub max_program_work_units: usize,
    pub max_allocated_bytes: usize,
}
impl Default for PageProgramSessionLimits {
    fn default() -> Self {
        Self {
            metric_policy: PageMetricPolicy::DeclaredOnly,
            fonts: FontResourceLimits::default(),
            forms: FormLimits::default(),
            images: ImageLimits::default(),
            max_page_attempts: 10_000,
            max_content_references: 100_000,
            max_encoded_bytes: 256 << 20,
            max_decoded_work_bytes: 512 << 20,
            max_joined_bytes: 256 << 20,
            max_parser_input_bytes: 512 << 20,
            max_parser_nodes: 20_000_000,
            max_parser_owned_bytes: 1 << 30,
            max_parser_tokens: 20_000_000,
            max_parser_operations: 10_000_000,
            max_prepared_fonts: 100_000,
            max_font_name_bytes: 16 << 20,
            max_program_operations: 10_000_000,
            max_program_work_units: 100_000_000,
            max_allocated_bytes: 1 << 30,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct PageProgramLimits {
    pub forms: FormLimits,
    /// Independent image decode/cache partition, retained across calls.
    pub images: ImageLimits,
    pub content: PageContentLimits,
    pub parser: ContentLimits,
    pub program: ProgramLimits,
    pub registry: TextFontLimits,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PageProgramSessionUsage {
    pub page_attempts: usize,
    pub content_references: usize,
    pub encoded_bytes: usize,
    pub decoded_work_bytes: usize,
    pub joined_bytes: usize,
    pub parser_input_bytes: usize,
    pub parser_nodes: usize,
    pub parser_owned_bytes: usize,
    pub parser_tokens: usize,
    pub parser_operations: usize,
    pub prepared_fonts: usize,
    pub font_name_bytes: usize,
    pub program_operations: usize,
    pub program_work_units: usize,
    pub allocated_bytes: usize,
}
#[derive(Debug)]
pub enum PageProgramErrorKind {
    Pages(Box<PageError>),
    Geometry(PageGeometryError),
    Content(Box<PageContentError>),
    Parser(Box<ContentError>),
    Operator(OperatorError),
    Font(Box<FontResourceError>),
    TextFont(TextFontError),
    Program(Box<ProgramError>),
    Form(Box<FormError>),
    FormStream(Box<StreamObjectError>),
    Image(Box<ImageError>),
    PageIndex,
    LimitExceeded(&'static str),
    Unsupported(OperatorKind),
    State(&'static str),
    Invariant(&'static str),
    AllocationFailed,
}
#[derive(Debug)]
pub struct PageProgramError {
    pub operation_index: Option<usize>,
    pub span: Option<Range<usize>>,
    pub kind: PageProgramErrorKind,
    pub source_object: Option<ObjectId>,
    /// Deepest failing Form first, followed by its callers toward the root page.
    pub diagnostic: Vec<FormDiagnosticFrame>,
}
impl fmt::Display for PageProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "page program source {:?} operation {:?} at {:?}: {:?}; Form chain {:?}",
            self.source_object, self.operation_index, self.span, self.kind, self.diagnostic
        )
    }
}
impl std::error::Error for PageProgramError {}
fn error(kind: PageProgramErrorKind) -> PageProgramError {
    PageProgramError {
        operation_index: None,
        span: None,
        kind,
        source_object: None,
        diagnostic: Vec::new(),
    }
}
fn at(mut error: PageProgramError, index: usize, span: Range<usize>) -> PageProgramError {
    error.operation_index = Some(index);
    error.span = Some(span);
    error
}
fn charge(
    value: &mut usize,
    amount: usize,
    cap: usize,
    label: &'static str,
) -> Result<(), PageProgramError> {
    *value = value
        .checked_add(amount)
        .filter(|n| *n <= cap)
        .ok_or_else(|| error(PageProgramErrorKind::LimitExceeded(label)))?;
    Ok(())
}
fn product(a: usize, b: usize) -> Result<usize, PageProgramError> {
    a.checked_mul(b)
        .ok_or_else(|| error(PageProgramErrorKind::LimitExceeded("allocation arithmetic")))
}

/// A page annotation's selected normal appearance stream and its Rect, in
/// default user space with x0 < x1 and y0 < y1. Callers decide visibility.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnnotationAppearance {
    pub annotation: ObjectId,
    pub stream: ObjectId,
    pub rect: [f64; 4],
}
/// ISO 32000-1 12.5.5: map the appearance BBox, transformed by its Matrix,
/// onto the annotation Rect; the Form's own BBox clip is applied afterwards.
fn annotation_matrix(
    matrix: Affine<UserSpace, UserSpace>,
    bbox: Rect<UserSpace>,
    rect: [f64; 4],
) -> Result<Affine<UserSpace, UserSpace>, PageProgramError> {
    let geometry = |e| {
        error(PageProgramErrorKind::Form(Box::new(FormError {
            object: None,
            field: "annotation appearance",
            kind: FormErrorKind::Geometry(e),
        })))
    };
    let [x0, y0, x1, y1] = matrix.transform_bounds(bbox).map_err(geometry)?.bounds();
    if x1 <= x0 || y1 <= y0 {
        return Err(error(PageProgramErrorKind::Invariant(
            "degenerate transformed appearance box",
        )));
    }
    let sx = (rect[2] - rect[0]) / (x1 - x0);
    let sy = (rect[3] - rect[1]) / (y1 - y0);
    let fit = Affine::from_pdf([sx, 0., 0., sy, rect[0] - sx * x0, rect[1] - sy * y0])
        .map_err(geometry)?;
    matrix.then(fit).map_err(geometry)
}
#[derive(Debug)]
pub struct PageFont {
    scope: ResourceScopeId,
    name: Vec<u8>,
    identity: FontIdentity,
    metric_kind: TextMetricKind,
    unicode_kind: TextUnicodeKind,
    first_operation: usize,
    first_span: Range<usize>,
}
impl PageFont {
    pub fn scope_id(&self) -> ResourceScopeId {
        self.scope
    }
    pub fn name(&self) -> &[u8] {
        &self.name
    }
    pub fn identity(&self) -> FontIdentity {
        self.identity
    }
    pub fn metric_kind(&self) -> TextMetricKind {
        self.metric_kind
    }
    pub fn unicode_kind(&self) -> TextUnicodeKind {
        self.unicode_kind
    }
    pub fn first_operation(&self) -> usize {
        self.first_operation
    }
}
/// A projection names the occurrence, never merely the repeated object ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentSourcePiece {
    Stream {
        occurrence: usize,
        object: ObjectId,
        decoded_range: Range<usize>,
        logical_range: Range<usize>,
    },
    Padding {
        occurrence: usize,
        logical_offset: usize,
    },
}
#[derive(Debug)]
pub struct PageProgram {
    page_index: usize,
    page_id: ObjectId,
    geometry: PageGeometry,
    metric_policy: PageMetricPolicy,
    content: PageContent,
    fonts: Vec<PageFont>,
    images: Vec<Arc<ImageAsset>>,
    program: Program,
    form_streams: Vec<FormStream>,
    form_invocations: Vec<FormInvocation>,
    resource_scopes: Vec<ResourceScope>,
    origins: Vec<OperationOrigin>,
}
impl PageProgram {
    /// Owned, result-local image assets indexed by each ImagePaint::image_id.
    pub fn images(&self) -> &[Arc<ImageAsset>] {
        &self.images
    }
    pub fn form_streams(&self) -> &[FormStream] {
        &self.form_streams
    }
    pub fn form_invocations(&self) -> &[FormInvocation] {
        &self.form_invocations
    }
    pub fn resource_scopes(&self) -> &[ResourceScope] {
        &self.resource_scopes
    }
    pub fn operation_origin(&self, index: usize) -> Option<&OperationOrigin> {
        self.origins.get(index)
    }
    pub fn page_index(&self) -> usize {
        self.page_index
    }
    pub fn page_id(&self) -> ObjectId {
        self.page_id
    }
    pub fn geometry(&self) -> PageGeometry {
        self.geometry
    }
    pub fn metric_policy(&self) -> PageMetricPolicy {
        self.metric_policy
    }
    pub fn content(&self) -> &PageContent {
        &self.content
    }
    pub fn fonts(&self) -> &[PageFont] {
        &self.fonts
    }
    pub fn program(&self) -> &Program {
        &self.program
    }
    /// Projects logical source spans, including synthetic padding. Decoded
    /// string payload positions are not raw-file or lexical string offsets.
    pub fn project_range(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = ContentSourcePiece> + '_> {
        if range.start > range.end || range.end > self.content.bytes.len() {
            return None;
        }
        Some(
            self.content
                .segments
                .iter()
                .enumerate()
                .flat_map(move |(occurrence, segment)| {
                    let start = range.start.max(segment.range.start);
                    let end = range.end.min(segment.range.end);
                    let stream = (start < end).then(|| ContentSourcePiece::Stream {
                        occurrence,
                        object: segment.object,
                        decoded_range: start - segment.range.start..end - segment.range.start,
                        logical_range: start..end,
                    });
                    let padding =
                        segment
                            .padding
                            .filter(|p| range.contains(p))
                            .map(|logical_offset| ContentSourcePiece::Padding {
                                occurrence,
                                logical_offset,
                            });
                    [stream, padding].into_iter().flatten()
                }),
        )
    }
}

pub struct PageProgramSession<'doc, 'input> {
    document: &'doc Document<'input>,
    pages: Vec<PageInfo>,
    geometry: Vec<Result<PageGeometry, PageGeometryError>>,
    loader: FontResourceLoader<'doc, 'input>,
    forms: FormResourceLoader<'doc, 'input>,
    images: ImageLoader<'doc, 'input>,
    limits: PageProgramSessionLimits,
    usage: PageProgramSessionUsage,
}
impl<'doc, 'input> PageProgramSession<'doc, 'input> {
    pub fn new(
        document: &'doc Document<'input>,
        limits: PageProgramSessionLimits,
    ) -> Result<Self, PageProgramError> {
        let pages = collect_pages(document, limits.fonts.pages)
            .map_err(|e| error(PageProgramErrorKind::Pages(Box::new(e))))?;
        let geometry = pages.iter().map(PageGeometry::from_page_info).collect();
        let loader = FontResourceLoader::from_page_ids(
            document,
            limits.fonts,
            pages.iter().map(|p| p.id).collect(),
        );
        Ok(Self {
            document,
            pages,
            geometry,
            loader,
            forms: FormResourceLoader::new(document, limits.forms),
            images: ImageLoader::new(document, limits.images),
            limits,
            usage: PageProgramSessionUsage::default(),
        })
    }
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }
    pub fn usage(&self) -> PageProgramSessionUsage {
        self.usage
    }
    pub fn form_usage(&self) -> FormUsage {
        self.forms.usage()
    }
    pub fn image_usage(&self) -> ImageUsage {
        self.images.usage()
    }
    pub fn font_usage(&self) -> FontResourceUsage {
        self.loader.usage()
    }
    pub fn trace_page(
        &mut self,
        page_index: usize,
        limits: PageProgramLimits,
    ) -> Result<PageProgram, PageProgramError> {
        self.trace_page_with_annotations(page_index, limits, &[])
    }
    /// Trace page content followed by the given annotation appearances, in
    /// order. Selection and visibility policy belong to the caller.
    pub fn trace_page_with_annotations(
        &mut self,
        page_index: usize,
        limits: PageProgramLimits,
        annotations: &[AnnotationAppearance],
    ) -> Result<PageProgram, PageProgramError> {
        self.forms.begin_page(limits.forms);
        self.images.begin_page(limits.images);
        let attempt_start = self.usage;
        charge(
            &mut self.usage.page_attempts,
            1,
            self.limits.max_page_attempts,
            "page attempts",
        )?;
        let page = self
            .pages
            .get(page_index)
            .ok_or_else(|| error(PageProgramErrorKind::PageIndex))?;
        let page_id = page.id;
        self.loader
            .validate_page_program_resources(page_id)
            .map_err(|e| error(PageProgramErrorKind::Font(Box::new(e))))?;
        let geometry =
            self.geometry[page_index].map_err(|e| error(PageProgramErrorKind::Geometry(e)))?;
        let mut content_limits = limits.content;
        charge(
            &mut self.usage.content_references,
            page.contents.len(),
            self.limits.max_content_references,
            "content references",
        )?;
        content_limits.max_referenced_encoded_bytes = content_limits
            .max_referenced_encoded_bytes
            .min(self.limits.max_encoded_bytes - self.usage.encoded_bytes);
        content_limits.max_decoded_work_bytes = content_limits
            .max_decoded_work_bytes
            .min(self.limits.max_decoded_work_bytes - self.usage.decoded_work_bytes);
        content_limits.max_joined_bytes = content_limits
            .max_joined_bytes
            .min(self.limits.max_joined_bytes - self.usage.joined_bytes);
        // Reserve full requests for geometric joined-byte growth plus fixed
        // segment storage. Empty pages need no decoding or output reservation.
        let segment_bytes = product(page.contents.len(), size_of::<ContentSegment>())?;
        let allocation_remaining = self.limits.max_allocated_bytes - self.usage.allocated_bytes;
        if segment_bytes > allocation_remaining {
            return Err(error(PageProgramErrorKind::LimitExceeded(
                "content allocation",
            )));
        }
        content_limits.max_joined_bytes = content_limits
            .max_joined_bytes
            .min((allocation_remaining - segment_bytes) / 3);
        if page.contents.is_empty() {
            content_limits.max_decoded_work_bytes = 0;
            content_limits.max_joined_bytes = 0;
            content_limits.max_referenced_encoded_bytes = 0;
        }
        let reserved_allocation = product(content_limits.max_joined_bytes, 3)? + segment_bytes;
        let encoded_reservation = content_limits.max_referenced_encoded_bytes;
        let decoded_reservation = content_limits.max_decoded_work_bytes;
        let joined_reservation = content_limits.max_joined_bytes;
        charge(
            &mut self.usage.allocated_bytes,
            reserved_allocation,
            self.limits.max_allocated_bytes,
            "content allocation",
        )?;
        charge(
            &mut self.usage.encoded_bytes,
            encoded_reservation,
            self.limits.max_encoded_bytes,
            "encoded bytes",
        )?;
        charge(
            &mut self.usage.decoded_work_bytes,
            decoded_reservation,
            self.limits.max_decoded_work_bytes,
            "decoded work",
        )?;
        charge(
            &mut self.usage.joined_bytes,
            joined_reservation,
            self.limits.max_joined_bytes,
            "joined bytes",
        )?;
        let mut content_allocation = 0;
        let content = load_page_content_with_allocation(
            self.document,
            page,
            content_limits,
            reserved_allocation,
            &mut content_allocation,
        )
        .map_err(|e| error(PageProgramErrorKind::Content(Box::new(e))))?;
        self.usage.encoded_bytes -= encoded_reservation - content.usage.encoded_bytes;
        self.usage.decoded_work_bytes -= decoded_reservation - content.usage.total_decoded_bytes;
        self.usage.joined_bytes -= joined_reservation - content.bytes.len();
        self.usage.allocated_bytes -= reserved_allocation - content_allocation;
        let mut plan = PreparedForms::default();
        let preparation = self.prepare_content(
            page_id,
            &content,
            annotations,
            limits,
            attempt_start,
            &mut plan,
        );
        let mut fonts = preparation.map_err(|e| plan.annotate(e, page_id))?;
        for font in &mut fonts {
            plan.current_source = Some(source_of(&plan.origins[font.first_operation]));
            plan.current_span = font.first_span.clone();
            let resource_error = |e| {
                at(
                    error(PageProgramErrorKind::Font(Box::new(e))),
                    font.first_operation,
                    font.first_span.clone(),
                )
            };
            let loaded = self
                .loader
                .load_scoped(
                    self.forms.scoped(plan.scope_sessions[font.scope.index()]),
                    &font.name,
                )
                .map_err(resource_error)
                .map_err(|e| plan.annotate(e, page_id))?;
            font.identity = loaded.identity();
            font.unicode_kind = match loaded.mapping() {
                SimpleFontMapping::ToUnicode(_) => TextUnicodeKind::ToUnicode,
                SimpleFontMapping::ExplicitEncoding { .. } => TextUnicodeKind::Encoding,
            };
            match self.loader.load_metrics_scoped(
                self.forms.scoped(plan.scope_sessions[font.scope.index()]),
                &font.name,
            ) {
                Ok(metrics) => {
                    if metrics.identity() != font.identity {
                        return Err(error(PageProgramErrorKind::Invariant(
                            "font/metrics identity",
                        )));
                    }
                }
                Err(e)
                    if matches!(
                        e.kind,
                        FontResourceErrorKind::NeedsBuiltinOrFontProgramMetrics
                    ) && self.limits.metric_policy == PageMetricPolicy::AllowPinnedBase14 =>
                {
                    let metrics = self
                        .loader
                        .load_builtin_metrics_scoped(
                            self.forms.scoped(plan.scope_sessions[font.scope.index()]),
                            &font.name,
                        )
                        .map_err(resource_error)
                        .map_err(|e| plan.annotate(e, page_id))?;
                    if metrics.identity() != font.identity {
                        return Err(error(PageProgramErrorKind::Invariant(
                            "font/builtin identity",
                        )));
                    }
                    font.metric_kind = TextMetricKind::Builtin(metrics.metrics().face());
                }
                Err(e) => return Err(plan.annotate(resource_error(e), page_id)),
            }
        }
        // No further mutable loader access: registry borrows a frozen cache.
        let entries_bytes = product(fonts.len(), size_of::<(&[u8], TextFont<'_>)>())?;
        charge(
            &mut self.usage.allocated_bytes,
            entries_bytes,
            self.limits.max_allocated_bytes,
            "font views",
        )?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(fonts.len())
            .map_err(|_| error(PageProgramErrorKind::AllocationFailed))?;
        for font in &fonts {
            let mapping = self
                .loader
                .loaded(font.identity)
                .ok_or_else(|| error(PageProgramErrorKind::Invariant("frozen font")))?
                .mapping();
            let view = match font.metric_kind {
                TextMetricKind::Declared => {
                    let unicode = match mapping {
                        SimpleFontMapping::ToUnicode(map) => UnicodeSource::ToUnicode(map),
                        SimpleFontMapping::ExplicitEncoding { encoding, .. } => {
                            UnicodeSource::Encoding(encoding)
                        }
                    };
                    TextFont::declared(
                        unicode,
                        self.loader
                            .loaded_metrics(font.identity)
                            .ok_or_else(|| {
                                error(PageProgramErrorKind::Invariant("frozen metrics"))
                            })?
                            .metrics(),
                    )
                }
                TextMetricKind::Builtin(_) => TextFont::builtin(
                    self.loader
                        .loaded_builtin_metrics(font.identity)
                        .ok_or_else(|| error(PageProgramErrorKind::Invariant("frozen builtin")))?
                        .metrics(),
                    match mapping {
                        SimpleFontMapping::ToUnicode(map) => Some(map),
                        _ => None,
                    },
                ),
            }
            .map_err(|e| {
                at(
                    error(PageProgramErrorKind::TextFont(e)),
                    font.first_operation,
                    font.first_span.clone(),
                )
            })?;
            entries.push((font.name.as_slice(), view));
        }
        let mut registry_limits = limits.registry;
        // Binding contains a Vec and a Copy TextFont; 128 is a conservative
        // platform-independent upper envelope on supported 32/64-bit targets.
        let registry_reservation = product(fonts.len(), 128)?
            .checked_add(fonts.iter().map(|f| f.name.len()).sum())
            .ok_or_else(|| error(PageProgramErrorKind::LimitExceeded("registry allocation")))?;
        registry_limits.max_capacity_bytes = registry_limits
            .max_capacity_bytes
            .min(registry_reservation)
            .min(self.limits.max_allocated_bytes - self.usage.allocated_bytes);
        charge(
            &mut self.usage.allocated_bytes,
            registry_limits.max_capacity_bytes,
            self.limits.max_allocated_bytes,
            "registry allocation",
        )?;
        let registry = TextFonts::new_indexed(&entries, registry_limits)
            .map_err(|e| error(PageProgramErrorKind::TextFont(e)))?;
        self.usage.allocated_bytes -=
            registry_limits.max_capacity_bytes - registry.capacity_bytes();
        // Field-separated borrows preserve the frozen font cache during replay.
        let program = execute_prepared(
            &content,
            &fonts,
            &registry,
            ExecutionConfig {
                geometry,
                limits,
                session: self.limits,
            },
            &mut self.usage,
            &mut self.forms,
            &mut plan,
        )
        .map_err(|e| plan.annotate(e, page_id))?;
        Ok(PageProgram {
            page_index,
            page_id,
            geometry,
            metric_policy: self.limits.metric_policy,
            content,
            fonts,
            images: plan.images,
            program,
            form_streams: plan.streams,
            form_invocations: plan.invocations,
            resource_scopes: plan.scopes,
            origins: plan.origins,
        })
    }
    fn reserve_vec<T>(&mut self, values: &mut Vec<T>, max: usize) -> Result<(), PageProgramError> {
        if values.len() < values.capacity() {
            return Ok(());
        }
        let capacity = values.capacity().saturating_mul(2).max(1).min(max);
        charge(
            &mut self.usage.allocated_bytes,
            product(capacity, size_of::<T>())?,
            self.limits.max_allocated_bytes,
            "descriptor allocation",
        )?;
        values
            .try_reserve_exact(capacity - values.len())
            .map_err(|_| error(PageProgramErrorKind::AllocationFailed))
    }
    fn reserve_parser(
        &mut self,
        len: usize,
        limits: ContentLimits,
    ) -> Result<ContentLimits, PageProgramError> {
        reserve_parser(&mut self.usage, self.limits, len, limits)
    }
    fn refund_parser(&mut self, limits: ContentLimits, actual: ContentUsage) {
        refund_parser(&mut self.usage, limits, actual);
    }
}
#[derive(Clone, Copy, Debug)]
enum SourceRef {
    Root,
    Form(FormInvocationId),
}
fn source_of(origin: &OperationOrigin) -> SourceRef {
    match origin {
        OperationOrigin::PageContent { .. } | OperationOrigin::PageScope { .. } => SourceRef::Root,
        OperationOrigin::FormContent { invocation, .. }
        | OperationOrigin::FormBoundary { invocation, .. } => SourceRef::Form(*invocation),
    }
}
struct PreparedImageCall {
    operation_index: usize,
    image_id: usize,
    object: ObjectId,
    scope: ResourceScopeId,
    name: Vec<u8>,
    hidden: bool,
}
#[derive(Default)]
struct PreparedForms {
    /// BDC /OC operations whose optional content is off.
    hidden_ops: std::collections::BTreeSet<usize>,
    /// Form invocations whose /OC is off.
    hidden_forms: std::collections::BTreeSet<usize>,
    /// cs/CS operations resolved to a device-equivalent component count.
    color_spaces: std::collections::BTreeMap<usize, usize>,
    images: Vec<Arc<ImageAsset>>,
    image_calls: Vec<PreparedImageCall>,
    image_diagnostic_reserved: bool,
    streams: Vec<FormStream>,
    invocations: Vec<FormInvocation>,
    scopes: Vec<ResourceScope>,
    scope_sessions: Vec<ResourceScopeId>,
    origins: Vec<OperationOrigin>,
    diagnostic: Vec<FormDiagnosticFrame>,
    current_source: Option<SourceRef>,
    current_span: Range<usize>,
}
impl PreparedForms {
    fn bytes<'a>(&'a self, content: &'a PageContent, source: SourceRef) -> &'a [u8] {
        match source {
            SourceRef::Root => &content.bytes,
            SourceRef::Form(id) => {
                self.streams[self.invocations[id.index()].stream.index()].bytes()
            }
        }
    }
    fn source_origin(source: SourceRef, span: Range<usize>) -> OperationOrigin {
        match source {
            SourceRef::Root => OperationOrigin::PageContent { span },
            SourceRef::Form(invocation) => OperationOrigin::FormContent { invocation, span },
        }
    }
    fn add_scope(
        &mut self,
        loader: &mut FormResourceLoader<'_, '_>,
        session: ResourceScopeId,
    ) -> Result<ResourceScopeId, PageProgramError> {
        loader.work(self.scope_sessions.len()).map_err(form_error)?;
        if let Some(i) = self.scope_sessions.iter().position(|&s| s == session) {
            return Ok(ResourceScopeId(i));
        }
        loader
            .reserve(
                &mut self.scope_sessions,
                loader.active_limits().max_resource_scopes,
            )
            .map_err(form_error)?;
        loader
            .reserve(&mut self.scopes, loader.active_limits().max_resource_scopes)
            .map_err(form_error)?;
        let id = ResourceScopeId(self.scopes.len());
        let mut descriptor = loader.scoped(session).descriptor.clone();
        descriptor.id = id;
        self.scopes.push(descriptor);
        self.scope_sessions.push(session);
        Ok(id)
    }
    fn add_origin(
        &mut self,
        loader: &mut FormResourceLoader<'_, '_>,
        origin: OperationOrigin,
    ) -> Result<(), PageProgramError> {
        loader.origin().map_err(form_error)?;
        loader
            .reserve(&mut self.origins, loader.active_limits().max_origin_records)
            .map_err(form_error)?;
        self.origins.push(origin);
        Ok(())
    }
    fn annotate(&mut self, mut error: PageProgramError, page: ObjectId) -> PageProgramError {
        let mut current = self.current_source.unwrap_or(SourceRef::Root);
        if error.span.is_none() {
            error.span = Some(self.current_span.clone());
        }
        let mut span = error.span.clone().expect("source span assigned");
        error.source_object = Some(match current {
            SourceRef::Root => page,
            SourceRef::Form(id) => self.invocations[id.index()].object,
        });
        self.diagnostic.clear();
        while let SourceRef::Form(id) = current {
            if self.diagnostic.len() == 64 || self.diagnostic.len() == self.diagnostic.capacity() {
                break;
            }
            let invocation = &self.invocations[id.index()];
            let (parent, call_span) = match &invocation.call_origin {
                OperationOrigin::PageContent { span } => (SourceRef::Root, span.clone()),
                OperationOrigin::FormContent { invocation, span } => {
                    (SourceRef::Form(*invocation), span.clone())
                }
                OperationOrigin::FormBoundary { .. } | OperationOrigin::PageScope { .. } => break,
            };
            let caller = match parent {
                SourceRef::Root => page,
                SourceRef::Form(p) => self.invocations[p.index()].object,
            };
            self.diagnostic.push(FormDiagnosticFrame {
                object: invocation.object,
                span,
                caller_object: caller,
                call_span: call_span.clone(),
            });
            span = call_span;
            current = parent;
        }
        error.diagnostic = std::mem::take(&mut self.diagnostic);
        error
    }
}
struct TraversalFrame {
    source: SourceRef,
    scope: ResourceScopeId,
    checkpoint: Option<ContentCheckpoint>,
    limits: ContentLimits,
    used: ContentUsage,
    preflight: Preflight,
    boundary: Option<FormFrame>,
}
impl TraversalFrame {
    fn new(
        source: SourceRef,
        scope: ResourceScopeId,
        bytes: &[u8],
        limits: ContentLimits,
        boundary: Option<FormFrame>,
    ) -> Self {
        Self {
            source,
            scope,
            checkpoint: Some(ContentParser::new(bytes, limits).into_checkpoint()),
            limits,
            used: ContentUsage::default(),
            preflight: Preflight::default(),
            boundary,
        }
    }
    fn next(&mut self, bytes: &[u8]) -> Result<Option<Operation>, PageProgramError> {
        let checkpoint = self
            .checkpoint
            .take()
            .ok_or_else(|| error(PageProgramErrorKind::Invariant("missing parser checkpoint")))?;
        let mut parser = ContentParser::resume(bytes, checkpoint)
            .map_err(|_| error(PageProgramErrorKind::Invariant("parser source changed")))?;
        let result = parser.next_operation();
        self.used = parser.usage();
        self.checkpoint = Some(parser.into_checkpoint());
        result.map_err(|e| {
            let offset = e.offset;
            at(
                error(PageProgramErrorKind::Parser(Box::new(e))),
                0,
                offset..offset,
            )
        })
    }
}
fn form_error(error_value: FormError) -> PageProgramError {
    error(PageProgramErrorKind::Form(Box::new(error_value)))
}
impl<'doc, 'input> PageProgramSession<'doc, 'input> {
    fn prepare_content(
        &mut self,
        page: ObjectId,
        content: &PageContent,
        annotations: &[AnnotationAppearance],
        limits: PageProgramLimits,
        start: PageProgramSessionUsage,
        plan: &mut PreparedForms,
    ) -> Result<Vec<PageFont>, PageProgramError> {
        let layers = OptionalContent::load(self.forms.document());
        let root_session = self.forms.root_scope(page).map_err(form_error)?;
        let root_scope = plan.add_scope(&mut self.forms, root_session)?;
        let mut frames = Vec::new();
        self.forms.reserve(&mut frames, 65).map_err(form_error)?;
        let parser_limits = self.reserve_parser(content.bytes.len(), limits.parser)?;
        frames.push(TraversalFrame::new(
            SourceRef::Root,
            root_scope,
            &content.bytes,
            parser_limits,
            None,
        ));
        if !annotations.is_empty() {
            plan.add_origin(
                &mut self.forms,
                OperationOrigin::PageScope {
                    phase: FormPhase::Enter,
                },
            )?;
        }
        let mut fonts = Vec::new();
        let mut selected_name_bytes = 0;
        let mut pending = annotations.iter();
        loop {
            if frames.is_empty() {
                // Annotation appearances draw after all page content, each as
                // a top-level Form invocation called from the page's end.
                let Some(annotation) = pending.next() else {
                    break;
                };
                let end = content.bytes.len();
                plan.current_source = Some(SourceRef::Root);
                plan.current_span = end..end;
                let index = plan.origins.len();
                (|| {
                    self.forms.invocation().map_err(form_error)?;
                    if plan.diagnostic.capacity() == 0 {
                        plan.diagnostic = self.forms.diagnostic().map_err(form_error)?;
                    }
                    charge(
                        &mut self.usage.content_references,
                        1,
                        self.limits.max_content_references,
                        "content references",
                    )?;
                    let definition = match self
                        .forms
                        .lookup_object(annotation.stream)
                        .map_err(form_error)?
                    {
                        XObjectDefinition::Form(definition) => definition,
                        XObjectDefinition::Image(_) => {
                            return Err(form_error(FormError {
                                object: Some(annotation.stream),
                                field: "AP",
                                kind: FormErrorKind::Invalid("appearance must be a Form"),
                            }));
                        }
                    };
                    self.invoke_form(
                        plan,
                        &mut frames,
                        definition,
                        (SourceRef::Root, root_scope),
                        end..end,
                        Some(*annotation),
                        limits,
                        start,
                    )
                })()
                .map_err(|e| {
                    if e.operation_index.is_some() {
                        e
                    } else {
                        at(e, index, end..end)
                    }
                })?;
                continue;
            }
            let frame = frames.last_mut().expect("frame");
            let source = frame.source;
            let scope = frame.scope;
            plan.current_source = Some(source);
            self.forms.work(1).map_err(form_error)?;
            let next = frame.next(plan.bytes(content, source)).map_err(|mut e| {
                e.operation_index = Some(plan.origins.len());
                e
            })?;
            let Some(operation) = next else {
                frame.preflight.finish().map_err(|e| {
                    at(
                        e,
                        plan.origins.len(),
                        plan.bytes(content, source).len()..plan.bytes(content, source).len(),
                    )
                })?;
                self.refund_parser(frame.limits, frame.used);
                frames.pop();
                if let SourceRef::Form(id) = source {
                    plan.add_origin(
                        &mut self.forms,
                        OperationOrigin::FormBoundary {
                            invocation: id,
                            phase: FormPhase::Leave,
                        },
                    )?;
                    plan.invocations[id.index()].execution_range.end = plan.origins.len();
                } else if !annotations.is_empty() {
                    plan.add_origin(
                        &mut self.forms,
                        OperationOrigin::PageScope {
                            phase: FormPhase::Leave,
                        },
                    )?;
                }
                continue;
            };
            let index = plan.origins.len();
            plan.current_span = operation.span.clone();
            let validated = validate_operation(&operation).map_err(|e| {
                at(
                    error(PageProgramErrorKind::Operator(e)),
                    index,
                    operation.span.clone(),
                )
            })?;
            if validated.kind() == OperatorKind::InlineImage {
                (|| {
                    frame.preflight.form_boundary()?;
                    let [
                        ContentOperand::Dictionary(entries),
                        ContentOperand::String(data),
                    ] = operation.operands.as_slice()
                    else {
                        unreachable!("validated BI")
                    };
                    let resources = self
                        .forms
                        .scoped(plan.scope_sessions[scope.index()])
                        .resources;
                    let dictionary = inline_dictionary(self.forms.document(), resources, entries);
                    // Synthetic identities sit above every real object number.
                    let object = ObjectId::new(u32::MAX - plan.image_calls.len() as u32, 0)
                        .expect("nonzero");
                    // Writers often omit the ASCII filters' end marker inside
                    // BI/EI, where EI already ends the data; MuPDF and pdf.js accept it.
                    let first_filter = match dictionary.get(b"Filter".as_slice()) {
                        Some(Object::Name(n)) => Some(n.as_slice()),
                        Some(Object::Array(items)) => match items.first() {
                            Some(Object::Name(n)) => Some(n.as_slice()),
                            _ => None,
                        },
                        _ => None,
                    };
                    let marker: &[u8] = match first_filter {
                        Some(b"ASCIIHexDecode") => b">",
                        Some(b"ASCII85Decode") => b"~>",
                        _ => b"",
                    };
                    let mut data = data.clone();
                    if !marker.is_empty() && !data.windows(marker.len()).any(|w| w == marker) {
                        data.extend_from_slice(marker);
                    }
                    let asset = self
                        .images
                        .load_inline(object, &dictionary, &data)
                        .map_err(|e| error(PageProgramErrorKind::Image(Box::new(e))))?;
                    self.forms
                        .reserve(&mut plan.images, limits.images.max_attempts)
                        .map_err(form_error)?;
                    let image_id = plan.images.len();
                    plan.images.push(asset);
                    self.forms
                        .reserve(&mut plan.image_calls, limits.images.max_attempts)
                        .map_err(form_error)?;
                    plan.image_calls.push(PreparedImageCall {
                        operation_index: index,
                        image_id,
                        object,
                        scope,
                        name: Vec::new(),
                        hidden: false,
                    });
                    plan.add_origin(
                        &mut self.forms,
                        PreparedForms::source_origin(source, operation.span.clone()),
                    )
                })()
                .map_err(|e| at(e, index, operation.span.clone()))?;
                continue;
            }
            if validated.kind() == OperatorKind::XObject {
                self.forms
                    .invocation()
                    .map_err(form_error)
                    .map_err(|e| at(e, index, operation.span.clone()))?;
                if plan.diagnostic.capacity() == 0 {
                    plan.diagnostic = self
                        .forms
                        .diagnostic()
                        .map_err(form_error)
                        .map_err(|e| at(e, index, operation.span.clone()))?;
                }
                frame
                    .preflight
                    .form_boundary()
                    .map_err(|e| at(e, index, operation.span.clone()))?;
                charge(
                    &mut self.usage.content_references,
                    1,
                    self.limits.max_content_references,
                    "content references",
                )?;
                let ContentOperand::Name(name) = &operation.operands[0] else {
                    unreachable!("validated Do")
                };
                let definition = self
                    .forms
                    .lookup(plan.scope_sessions[scope.index()], name)
                    .map_err(form_error)
                    .map_err(|e| at(e, index, operation.span.clone()))?;
                let definition = match definition {
                    XObjectDefinition::Image(object) => {
                        (|| {
                            if !plan.image_diagnostic_reserved {
                                // Only one image error can leave this page attempt. Reserve
                                // its owned box before any loader failure can allocate it.
                                self.forms
                                    .allocation(size_of::<ImageError>())
                                    .map_err(|_| {
                                        error(PageProgramErrorKind::LimitExceeded(
                                            "image diagnostic allocation",
                                        ))
                                    })?;
                                plan.image_diagnostic_reserved = true;
                            }
                            let asset = self.images.load(object).map_err(|e| {
                                at(
                                    error(PageProgramErrorKind::Image(Box::new(e))),
                                    index,
                                    operation.span.clone(),
                                )
                            })?;
                            self.forms.work(plan.images.len()).map_err(form_error)?;
                            let image_id = if let Some(id) =
                                plan.images.iter().position(|a| a.object_id() == object)
                            {
                                id
                            } else {
                                self.forms
                                    .reserve(&mut plan.images, limits.images.max_attempts)
                                    .map_err(form_error)?;
                                let id = plan.images.len();
                                plan.images.push(asset);
                                id
                            };
                            self.forms
                                .reserve(&mut plan.image_calls, limits.images.max_attempts)
                                .map_err(form_error)?;
                            self.forms.allocation(name.len()).map_err(form_error)?;
                            let mut owned_name = Vec::new();
                            owned_name
                                .try_reserve_exact(name.len())
                                .map_err(|_| error(PageProgramErrorKind::AllocationFailed))?;
                            owned_name.extend_from_slice(name);
                            let hidden = !object_visible(&layers, self.forms.document(), object);
                            plan.image_calls.push(PreparedImageCall {
                                operation_index: index,
                                image_id,
                                object,
                                scope,
                                name: owned_name,
                                hidden,
                            });
                            plan.add_origin(
                                &mut self.forms,
                                PreparedForms::source_origin(source, operation.span.clone()),
                            )?;
                            Ok(())
                        })()
                        .map_err(|e| at(e, index, operation.span.clone()))?;
                        continue;
                    }
                    XObjectDefinition::Form(definition) => definition,
                };
                let form_object = self.forms.definition(definition).object;
                self.invoke_form(
                    plan,
                    &mut frames,
                    definition,
                    (source, scope),
                    operation.span.clone(),
                    None,
                    limits,
                    start,
                )
                .map_err(|e| {
                    if e.operation_index.is_some() {
                        e
                    } else {
                        at(e, index, operation.span.clone())
                    }
                })?;
                if !object_visible(&layers, self.forms.document(), form_object) {
                    plan.hidden_forms.insert(plan.invocations.len() - 1);
                }
                continue;
            }
            frame
                .preflight
                .push(&validated, limits.program.max_save_depth)
                .map_err(|e| at(e, index, operation.span.clone()))?;
            if matches!(
                validated.kind(),
                OperatorKind::FillColorSpace | OperatorKind::StrokeColorSpace
            ) {
                let ContentOperand::Name(name) = &operation.operands[0] else {
                    unreachable!("validated cs")
                };
                let resources = self
                    .forms
                    .scoped(plan.scope_sessions[scope.index()])
                    .resources;
                if let Some(n) = color_space_components(self.forms.document(), resources, name) {
                    plan.color_spaces.insert(index, n);
                }
            }
            if crate::program::optional_content(&validated) {
                let doc = self.forms.document();
                let resources = self
                    .forms
                    .scoped(plan.scope_sessions[scope.index()])
                    .resources;
                // Inline property lists cannot name a group; they stay visible.
                let visible = match &operation.operands[1] {
                    ContentOperand::Name(name) => resources
                        .and_then(|r| r.get(b"Properties".as_slice()))
                        .and_then(|p| match p {
                            Object::Reference(id) => doc.object(*id).ok().map(|o| &o.value),
                            other => Some(other),
                        })
                        .and_then(|p| match p {
                            Object::Dictionary(d) => d.get(name.as_slice()),
                            _ => None,
                        })
                        .is_none_or(|value| layers.visible(doc, value)),
                    _ => true,
                };
                if !visible {
                    plan.hidden_ops.insert(index);
                }
            }
            if validated.kind() == OperatorKind::Font {
                let ContentOperand::Name(name) = &operation.operands[0] else {
                    unreachable!("validated Tf")
                };
                self.forms
                    .work(fonts.len().checked_mul(name.len() + 1).ok_or_else(|| {
                        error(PageProgramErrorKind::LimitExceeded(
                            "font lookup arithmetic",
                        ))
                    })?)
                    .map_err(form_error)?;
                if !fonts
                    .iter()
                    .any(|f: &PageFont| f.scope == scope && f.name == *name)
                {
                    if fonts.len() >= limits.registry.max_fonts
                        || name.len() > limits.registry.max_name_bytes
                    {
                        return Err(at(
                            error(PageProgramErrorKind::LimitExceeded("selected fonts")),
                            index,
                            operation.span,
                        ));
                    }
                    charge(
                        &mut selected_name_bytes,
                        name.len(),
                        limits.registry.max_total_name_bytes,
                        "selected name bytes",
                    )?;
                    charge(
                        &mut self.usage.prepared_fonts,
                        1,
                        self.limits.max_prepared_fonts,
                        "prepared fonts",
                    )?;
                    charge(
                        &mut self.usage.font_name_bytes,
                        name.len(),
                        self.limits.max_font_name_bytes,
                        "font name bytes",
                    )?;
                    self.reserve_vec(&mut fonts, limits.registry.max_fonts)?;
                    charge(
                        &mut self.usage.allocated_bytes,
                        name.len(),
                        self.limits.max_allocated_bytes,
                        "font names allocation",
                    )?;
                    let mut owned = Vec::new();
                    owned
                        .try_reserve_exact(name.len())
                        .map_err(|_| error(PageProgramErrorKind::AllocationFailed))?;
                    owned.extend_from_slice(name);
                    fonts.push(PageFont {
                        scope,
                        name: owned,
                        identity: FontIdentity::Indirect(page),
                        metric_kind: TextMetricKind::Declared,
                        unicode_kind: TextUnicodeKind::Encoding,
                        first_operation: index,
                        first_span: operation.span.clone(),
                    });
                }
            }
            plan.add_origin(
                &mut self.forms,
                PreparedForms::source_origin(source, operation.span),
            )?;
        }
        Ok(fonts)
    }
    /// Register one Form invocation and push its traversal frame. `Do`
    /// supplies the caller's source; annotations supply their Rect so the
    /// appearance is mapped per ISO 32000-1 12.5.5 (Algorithm 8.1).
    #[allow(clippy::too_many_arguments)]
    fn invoke_form(
        &mut self,
        plan: &mut PreparedForms,
        frames: &mut Vec<TraversalFrame>,
        definition: usize,
        (source, scope): (SourceRef, ResourceScopeId),
        call_span: Range<usize>,
        annotation: Option<AnnotationAppearance>,
        limits: PageProgramLimits,
        start: PageProgramSessionUsage,
    ) -> Result<(), PageProgramError> {
        if frames.len() > self.forms.active_limits().max_depth {
            return Err(error(PageProgramErrorKind::LimitExceeded("form depth")));
        }
        let meta = self.forms.definition(definition).clone();
        let index = plan.origins.len();
        let matrix = match annotation {
            None => meta.matrix,
            Some(a) => annotation_matrix(meta.matrix, meta.bbox, a.rect)?,
        };
        self.forms.work(frames.len()).map_err(form_error)?;
        if frames.iter().any(|f|matches!(f.source,SourceRef::Form(id) if plan.invocations[id.index()].object==meta.object)){return Err(error(PageProgramErrorKind::Form(Box::new(FormError{object:Some(meta.object),field:"Do",kind:crate::form_resources::FormErrorKind::Cycle(meta.object)}))));}
        let bytes = if let Some(bytes) = meta.bytes {
            bytes
        } else {
            self.decode_form(definition, meta.object, limits, start)?
        };
        if bytes.len() > self.forms.active_limits().max_stream_decoded_bytes {
            return Err(error(PageProgramErrorKind::LimitExceeded(
                "form decoded stream bytes",
            )));
        }
        self.forms.work(plan.streams.len()).map_err(form_error)?;
        let stream = if let Some(i) = plan.streams.iter().position(|s| s.object == meta.object) {
            FormStreamId(i)
        } else {
            self.forms
                .reserve(
                    &mut plan.streams,
                    self.forms.active_limits().max_unique_streams,
                )
                .map_err(form_error)?;
            let id = FormStreamId(plan.streams.len());
            plan.streams.push(FormStream {
                object: meta.object,
                bytes,
            });
            id
        };
        let effective = plan.add_scope(
            &mut self.forms,
            meta.scope.unwrap_or(plan.scope_sessions[scope.index()]),
        )?;
        self.forms
            .reserve(
                &mut plan.invocations,
                self.forms.active_limits().max_invocations,
            )
            .map_err(form_error)?;
        let id = FormInvocationId(plan.invocations.len());
        let parent = match source {
            SourceRef::Root => None,
            SourceRef::Form(id) => Some(id),
        };
        plan.invocations.push(FormInvocation {
            id,
            parent,
            stream,
            object: meta.object,
            caller_scope: scope,
            effective_scope: effective,
            call_origin: PreparedForms::source_origin(source, call_span),
            matrix,
            bbox: meta.bbox,
            execution_range: index..index,
            annotation: annotation.map(|a| a.annotation),
        });
        plan.add_origin(
            &mut self.forms,
            OperationOrigin::FormBoundary {
                invocation: id,
                phase: FormPhase::Enter,
            },
        )?;
        self.forms.reserve(frames, 65).map_err(form_error)?;
        let child_bytes = plan.streams[stream.index()].bytes();
        let child_limits = self.reserve_parser(child_bytes.len(), limits.parser)?;
        frames.push(TraversalFrame::new(
            SourceRef::Form(id),
            effective,
            child_bytes,
            child_limits,
            None,
        ));
        Ok(())
    }
    fn decode_form(
        &mut self,
        index: usize,
        object: ObjectId,
        limits: PageProgramLimits,
        start: PageProgramSessionUsage,
    ) -> Result<Arc<Vec<u8>>, PageProgramError> {
        self.forms.meter_decoder(index).map_err(form_error)?;
        let encoded = self
            .document
            .stream_data(object)
            .map_err(|e| {
                error(PageProgramErrorKind::FormStream(Box::new(
                    StreamObjectError {
                        object,
                        kind: crate::stream_object::StreamObjectErrorKind::Document(e),
                    },
                )))
            })?
            .ok_or_else(|| error(PageProgramErrorKind::Invariant("form stream missing")))?
            .len();
        let mut decoder = limits.content.stream;
        decoder.max_filters = decoder.max_filters.min(8);
        decoder.max_encoded_bytes = decoder
            .max_encoded_bytes
            .min(self.limits.max_encoded_bytes - self.usage.encoded_bytes)
            .min(
                limits
                    .content
                    .max_referenced_encoded_bytes
                    .saturating_sub(self.usage.encoded_bytes - start.encoded_bytes),
            );
        if encoded > decoder.max_encoded_bytes {
            return Err(error(PageProgramErrorKind::LimitExceeded(
                "form encoded bytes",
            )));
        }
        decoder.max_decoded_bytes = decoder
            .max_decoded_bytes
            .min(self.forms.active_limits().max_stream_decoded_bytes);
        decoder.max_total_decoded_bytes = decoder
            .max_total_decoded_bytes
            .min(self.limits.max_decoded_work_bytes - self.usage.decoded_work_bytes)
            .min(
                limits
                    .content
                    .max_decoded_work_bytes
                    .saturating_sub(self.usage.decoded_work_bytes - start.decoded_work_bytes),
            );
        // Existing Flate starts with <=4096 spare bytes and grows geometrically.
        // Six times all stage output plus 12288 per possible stage bounds full
        // allocation requests, including small/empty outputs and capped growth.
        let stages = decoder
            .max_filters
            .checked_mul(2)
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| error(PageProgramErrorKind::LimitExceeded("decoder stages")))?;
        let overhead = product(stages, 12_288)?;
        let remaining = self.limits.max_allocated_bytes - self.usage.allocated_bytes;
        if overhead > remaining {
            return Err(error(PageProgramErrorKind::LimitExceeded(
                "form allocation",
            )));
        }
        decoder.max_total_decoded_bytes = decoder
            .max_total_decoded_bytes
            .min((remaining - overhead) / 6);
        let reservation = product(decoder.max_total_decoded_bytes, 6)?
            .checked_add(overhead)
            .ok_or_else(|| error(PageProgramErrorKind::LimitExceeded("form allocation")))?;
        charge(
            &mut self.usage.encoded_bytes,
            encoded,
            self.limits.max_encoded_bytes,
            "form encoded bytes",
        )?;
        charge(
            &mut self.usage.decoded_work_bytes,
            decoder.max_total_decoded_bytes,
            self.limits.max_decoded_work_bytes,
            "form decoded work",
        )?;
        charge(
            &mut self.usage.allocated_bytes,
            reservation,
            self.limits.max_allocated_bytes,
            "form decode allocation",
        )?;
        let decoded = decode_document_stream_with_usage(self.document, object, decoder)
            .map_err(|e| error(PageProgramErrorKind::FormStream(Box::new(e))))?;
        self.usage.decoded_work_bytes -=
            decoder.max_total_decoded_bytes - decoded.usage.total_decoded_bytes;
        self.usage.allocated_bytes -=
            reservation - (product(decoded.usage.total_decoded_bytes, 6)? + overhead);
        self.forms
            .allocation(size_of::<Vec<u8>>() + 2 * size_of::<usize>())
            .map_err(form_error)?;
        let bytes = Arc::new(decoded.bytes);
        self.forms.set_bytes(index, Arc::clone(&bytes));
        Ok(bytes)
    }
}
struct ExecutionConfig {
    geometry: PageGeometry,
    limits: PageProgramLimits,
    session: PageProgramSessionLimits,
}
fn execute_prepared(
    content: &PageContent,
    fonts: &[PageFont],
    registry: &TextFonts<'_>,
    config: ExecutionConfig,
    usage: &mut PageProgramSessionUsage,
    forms: &mut FormResourceLoader<'_, '_>,
    plan: &mut PreparedForms,
) -> Result<Program, PageProgramError> {
    let ExecutionConfig {
        geometry,
        limits,
        session,
    } = config;
    let mut program_limits = limits.program;
    program_limits.max_operations = program_limits
        .max_operations
        .min(session.max_program_operations - usage.program_operations);
    program_limits.max_work_units = program_limits
        .max_work_units
        .min(session.max_program_work_units - usage.program_work_units);
    program_limits.max_allocated_bytes = program_limits
        .max_allocated_bytes
        .min(session.max_allocated_bytes - usage.allocated_bytes);
    let mut interpreter = ProgramInterpreter::new(
        registry,
        ProgramInitialState::new(
            geometry.user_to_page(),
            InitialClip::Rectangle(geometry.bounds()),
        ),
        program_limits,
    )
    .map_err(program_error)?;
    let execution = (|| {
        let mut image_cursor = 0;
        let mut frames = Vec::new();
        forms.reserve(&mut frames, 65).map_err(form_error)?;
        let root_limits = reserve_parser(usage, session, content.bytes.len(), limits.parser)?;
        let page_scope = match plan.origins.first() {
            Some(OperationOrigin::PageScope {
                phase: FormPhase::Enter,
            }) => Some(
                interpreter
                    .enter_page_scope()
                    .map_err(|e| at(program_error(e), 0, 0..0))?,
            ),
            _ => None,
        };
        frames.push(TraversalFrame::new(
            SourceRef::Root,
            ResourceScopeId(0),
            &content.bytes,
            root_limits,
            page_scope,
        ));
        let mut next_annotation = 0;
        loop {
            if frames.is_empty() {
                let Some(position) = plan.invocations[next_annotation..]
                    .iter()
                    .position(|i| i.annotation.is_some())
                else {
                    break;
                };
                let id = FormInvocationId(next_annotation + position);
                next_annotation = id.index() + 1;
                forms.work(position + 1).map_err(form_error)?;
                let index = interpreter.usage().operations;
                let end = content.bytes.len();
                if plan.origins.get(index)
                    != Some(&OperationOrigin::FormBoundary {
                        invocation: id,
                        phase: FormPhase::Enter,
                    })
                {
                    return Err(error(PageProgramErrorKind::Invariant(
                        "annotation entry plan",
                    )));
                }
                plan.current_source = Some(SourceRef::Root);
                plan.current_span = end..end;
                let invocation = &plan.invocations[id.index()];
                let boundary = interpreter
                    .enter_form(invocation.matrix, invocation.bbox)
                    .map_err(|e| at(program_error(e), index, end..end))?;
                let child_source = SourceRef::Form(id);
                let child_scope = invocation.effective_scope;
                forms.reserve(&mut frames, 65).map_err(form_error)?;
                let bytes = plan.bytes(content, child_source);
                let child_limits = reserve_parser(usage, session, bytes.len(), limits.parser)?;
                frames.push(TraversalFrame::new(
                    child_source,
                    child_scope,
                    bytes,
                    child_limits,
                    Some(boundary),
                ));
                continue;
            }
            let frame = frames.last_mut().expect("frame");
            let source = frame.source;
            let scope = frame.scope;
            let index = interpreter.usage().operations;
            plan.current_source = Some(source);
            forms.work(1).map_err(form_error)?;
            let next = frame.next(plan.bytes(content, source)).map_err(|mut e| {
                e.operation_index = Some(index);
                e
            })?;
            let Some(operation) = next else {
                refund_parser(usage, frame.limits, frame.used);
                let mut frame = frames.pop().expect("frame");
                if let Some(boundary) = frame.boundary.take() {
                    let expected = match source {
                        SourceRef::Form(invocation) => OperationOrigin::FormBoundary {
                            invocation,
                            phase: FormPhase::Leave,
                        },
                        SourceRef::Root => OperationOrigin::PageScope {
                            phase: FormPhase::Leave,
                        },
                    };
                    if plan.origins.get(index) != Some(&expected) {
                        return Err(error(PageProgramErrorKind::Invariant("Form exit plan")));
                    }
                    let end = plan.bytes(content, source).len();
                    interpreter
                        .leave_form(boundary)
                        .map_err(|e| at(program_error(e), index, end..end))?;
                    if matches!(source, SourceRef::Form(id) if plan.hidden_forms.contains(&id.index()))
                    {
                        interpreter.end_hidden();
                    }
                }
                continue;
            };
            plan.current_span = operation.span.clone();
            if matches!(operation.operator.as_slice(), b"TJ" | b"d") {
                if let Some(ContentOperand::Array(items)) = operation.operands.first() {
                    if items.len()
                        > program_limits
                            .max_work_units
                            .saturating_sub(interpreter.usage().work_units)
                    {
                        return Err(at(
                            error(PageProgramErrorKind::LimitExceeded("program work units")),
                            index,
                            operation.span,
                        ));
                    }
                }
            }
            let validated = validate_operation(&operation).map_err(|e| {
                at(
                    error(PageProgramErrorKind::Operator(e)),
                    index,
                    operation.span.clone(),
                )
            })?;
            if validated.kind() == OperatorKind::InlineImage {
                let call = plan
                    .image_calls
                    .get(image_cursor)
                    .filter(|call| call.operation_index == index && call.name.is_empty())
                    .ok_or_else(|| error(PageProgramErrorKind::Invariant("inline image plan")))?;
                if call.scope != scope
                    || plan.origins.get(index)
                        != Some(&PreparedForms::source_origin(
                            source,
                            operation.span.clone(),
                        ))
                {
                    return Err(error(PageProgramErrorKind::Invariant(
                        "inline image callsite changed",
                    )));
                }
                interpreter
                    .push_image(call.image_id, &operation)
                    .map_err(program_error)?;
                image_cursor += 1;
                continue;
            }
            if validated.kind() == OperatorKind::XObject {
                if let Some(call) = plan
                    .image_calls
                    .get(image_cursor)
                    .filter(|call| call.operation_index == index)
                {
                    let ContentOperand::Name(name) = &operation.operands[0] else {
                        unreachable!("validated Do")
                    };
                    forms
                        .work(name.len().checked_add(1).ok_or_else(|| {
                            error(PageProgramErrorKind::LimitExceeded("image call work"))
                        })?)
                        .map_err(form_error)?;
                    if call.scope != scope
                        || call.name != *name
                        || plan.images.get(call.image_id).map(|a| a.object_id())
                            != Some(call.object)
                        || plan.origins.get(index)
                            != Some(&PreparedForms::source_origin(
                                source,
                                operation.span.clone(),
                            ))
                    {
                        return Err(error(PageProgramErrorKind::Invariant(
                            "image callsite changed",
                        )));
                    }
                    if call.hidden {
                        interpreter.begin_hidden();
                    }
                    let pushed = interpreter.push_image(call.image_id, &operation);
                    if call.hidden {
                        interpreter.end_hidden();
                    }
                    pushed.map_err(program_error)?;
                    image_cursor += 1;
                    continue;
                }
                let Some(OperationOrigin::FormBoundary {
                    invocation: id,
                    phase: FormPhase::Enter,
                }) = plan.origins.get(index)
                else {
                    return Err(error(PageProgramErrorKind::Invariant("Form entry plan")));
                };
                let id = *id;
                let invocation = &plan.invocations[id.index()];
                if invocation.call_origin != PreparedForms::source_origin(source, operation.span)
                    || invocation.caller_scope != scope
                {
                    return Err(error(PageProgramErrorKind::Invariant(
                        "Form callsite changed",
                    )));
                }
                let boundary = interpreter
                    .enter_form(invocation.matrix, invocation.bbox)
                    .map_err(|e| at(program_error(e), index, plan.current_span.clone()))?;
                if plan.hidden_forms.contains(&id.index()) {
                    interpreter.begin_hidden();
                }
                let child_source = SourceRef::Form(id);
                let child_scope = invocation.effective_scope;
                forms.reserve(&mut frames, 65).map_err(form_error)?;
                let bytes = plan.bytes(content, child_source);
                let child_limits = reserve_parser(usage, session, bytes.len(), limits.parser)?;
                frames.push(TraversalFrame::new(
                    child_source,
                    child_scope,
                    bytes,
                    child_limits,
                    Some(boundary),
                ));
                continue;
            }
            if plan.origins.get(index)
                != Some(&PreparedForms::source_origin(
                    source,
                    operation.span.clone(),
                ))
            {
                return Err(error(PageProgramErrorKind::Invariant(
                    "operation source changed",
                )));
            }
            if validated.kind() == OperatorKind::Font {
                let ContentOperand::Name(name) = &operation.operands[0] else {
                    unreachable!()
                };
                forms
                    .work(fonts.len().checked_mul(name.len() + 1).ok_or_else(|| {
                        error(PageProgramErrorKind::LimitExceeded(
                            "font lookup arithmetic",
                        ))
                    })?)
                    .map_err(form_error)?;
                let index = fonts
                    .iter()
                    .position(|f| f.scope == scope && f.name == *name)
                    .ok_or_else(|| {
                        error(PageProgramErrorKind::Invariant("prepared font missing"))
                    })?;
                interpreter
                    .push_resolved_font(&validated, TextFontId::from_index(index))
                    .map_err(program_error)?;
            } else if let Some(&n) = plan.color_spaces.get(&index) {
                interpreter
                    .push_color_space(&validated, n)
                    .map_err(program_error)?;
            } else if crate::program::optional_content(&validated) {
                interpreter
                    .push_optional_content(&validated, !plan.hidden_ops.contains(&index))
                    .map_err(program_error)?;
            } else {
                interpreter.push(&validated).map_err(program_error)?;
            }
        }
        if image_cursor != plan.image_calls.len() {
            return Err(error(PageProgramErrorKind::Invariant(
                "image execution length",
            )));
        }
        if interpreter.usage().operations != plan.origins.len() {
            return Err(error(PageProgramErrorKind::Invariant("execution length")));
        }
        Ok(())
    })();
    let used = interpreter.usage();
    usage.program_operations += used.operations;
    usage.program_work_units += used.work_units;
    usage.allocated_bytes += used.allocated_bytes;
    execution?;
    interpreter.finish().map_err(program_error)
}

/// Expand inline-image abbreviations (ISO 32000-1 Tables 92-94) into an
/// ordinary image dictionary. A named color space comes from the resources.
fn inline_dictionary(
    doc: &Document<'_>,
    resources: Option<&Dictionary>,
    entries: &std::collections::BTreeMap<Vec<u8>, ContentOperand>,
) -> Dictionary {
    fn object(value: &ContentOperand) -> Object {
        match value {
            ContentOperand::Null => Object::Null,
            ContentOperand::Boolean(v) => Object::Boolean(*v),
            ContentOperand::Integer(v) => Object::Integer(*v),
            ContentOperand::Real(v) => Object::Real(*v),
            ContentOperand::Name(v) => Object::Name(v.clone()),
            ContentOperand::String(v) => Object::String(v.clone()),
            ContentOperand::Array(v) => Object::Array(v.iter().map(object).collect()),
            ContentOperand::Dictionary(v) => {
                Object::Dictionary(v.iter().map(|(k, v)| (k.clone(), object(v))).collect())
            }
        }
    }
    let name = |n: &[u8]| -> Vec<u8> {
        match n {
            b"G" => b"DeviceGray".to_vec(),
            b"RGB" => b"DeviceRGB".to_vec(),
            b"CMYK" => b"DeviceCMYK".to_vec(),
            b"I" => b"Indexed".to_vec(),
            b"AHx" => b"ASCIIHexDecode".to_vec(),
            b"A85" => b"ASCII85Decode".to_vec(),
            b"LZW" => b"LZWDecode".to_vec(),
            b"Fl" => b"FlateDecode".to_vec(),
            b"RL" => b"RunLengthDecode".to_vec(),
            b"CCF" => b"CCITTFaxDecode".to_vec(),
            b"DCT" => b"DCTDecode".to_vec(),
            other => other.to_vec(),
        }
    };
    let expand = |value: Object| match value {
        Object::Name(n) => Object::Name(name(&n)),
        Object::Array(items) => Object::Array(
            items
                .into_iter()
                .map(|v| match v {
                    Object::Name(n) => Object::Name(name(&n)),
                    other => other,
                })
                .collect(),
        ),
        other => other,
    };
    let mut d = Dictionary::new();
    d.insert(b"Subtype".to_vec(), Object::Name(b"Image".to_vec()));
    for (key, value) in entries {
        let key: &[u8] = match key.as_slice() {
            b"BPC" => b"BitsPerComponent",
            b"CS" => b"ColorSpace",
            b"D" => b"Decode",
            b"DP" => b"DecodeParms",
            b"F" => b"Filter",
            b"H" => b"Height",
            b"IM" => b"ImageMask",
            b"I" => b"Interpolate",
            b"W" => b"Width",
            b"L" | b"Length" => continue,
            other => other,
        };
        let mut value = object(value);
        if matches!(key, b"ColorSpace" | b"Filter") {
            value = expand(value);
        }
        if key == b"ColorSpace" {
            if let Object::Name(n) = &value {
                if !n.starts_with(b"Device") && n.as_slice() != b"Indexed" {
                    let named = resources
                        .and_then(|r| r.get(b"ColorSpace".as_slice()))
                        .and_then(|c| match c {
                            Object::Reference(id) => doc.object(*id).ok().map(|o| &o.value),
                            other => Some(other),
                        })
                        .and_then(|c| match c {
                            Object::Dictionary(c) => c.get(n.as_slice()),
                            _ => None,
                        });
                    if let Some(named) = named {
                        value = match named {
                            Object::Reference(id) => doc
                                .object(*id)
                                .map(|o| o.value.clone())
                                .unwrap_or(Object::Null),
                            other => other.clone(),
                        };
                    }
                }
            }
        }
        d.insert(key.to_vec(), value);
    }
    d
}

/// Component count of a color space that renders as a device space: the
/// device families, CalGray/CalRGB and ICCBased (by /N). Lab, Indexed,
/// Separation, DeviceN and Pattern return None and stay refused.
/// ponytail: Cal and ICC colors are drawn as device values without profile
/// conversion; add color management when a document's colors depend on it.
fn color_space_components(
    doc: &Document<'_>,
    resources: Option<&Dictionary>,
    name: &[u8],
) -> Option<usize> {
    let resolve = |value: &Object| -> Option<Object> {
        match value {
            Object::Reference(id) => doc.object(*id).ok().map(|o| o.value.clone()),
            other => Some(other.clone()),
        }
    };
    let device = |n: &[u8]| match n {
        b"DeviceGray" | b"G" => Some(1),
        b"DeviceRGB" | b"RGB" => Some(3),
        b"DeviceCMYK" | b"CMYK" => Some(4),
        _ => None,
    };
    if let Some(n) = device(name) {
        return Some(n);
    }
    let spaces = resolve(resources?.get(b"ColorSpace".as_slice())?)?;
    let Object::Dictionary(spaces) = spaces else {
        return None;
    };
    match resolve(spaces.get(name)?)? {
        Object::Name(n) => device(&n),
        Object::Array(items) => match items.first()? {
            Object::Name(family) if family == b"CalGray" => Some(1),
            Object::Name(family) if family == b"CalRGB" => Some(3),
            Object::Name(family) if family == b"ICCBased" => {
                let Some(Object::Dictionary(stream)) = items.get(1).and_then(resolve) else {
                    return None;
                };
                match stream.get(b"N".as_slice()).and_then(resolve) {
                    Some(Object::Integer(n @ (1 | 3 | 4))) => Some(n as usize),
                    _ => None,
                }
            }
            Object::Name(n) => device(n),
            _ => None,
        },
        _ => None,
    }
}

/// An XObject draws unless its /OC group or membership is off.
fn object_visible(layers: &OptionalContent, doc: &Document<'_>, id: ObjectId) -> bool {
    match doc.object(id).map(|o| &o.value) {
        Ok(Object::Dictionary(d)) => d
            .get(b"OC".as_slice())
            .is_none_or(|oc| layers.visible(doc, oc)),
        _ => true,
    }
}
fn program_error(e: ProgramError) -> PageProgramError {
    let index = e.operation_index;
    let span = e.span.clone();
    at(
        error(PageProgramErrorKind::Program(Box::new(e))),
        index,
        span,
    )
}
fn reserve_parser(
    usage: &mut PageProgramSessionUsage,
    session: PageProgramSessionLimits,
    len: usize,
    mut limits: ContentLimits,
) -> Result<ContentLimits, PageProgramError> {
    charge(
        &mut usage.parser_input_bytes,
        len,
        session.max_parser_input_bytes,
        "parser input bytes",
    )?;
    // A source byte can produce at most one node, payload byte, token or op.
    // The lexical temporary is also capped by source length before allocation.
    limits.max_nodes = limits
        .max_nodes
        .min(len)
        .min(session.max_parser_nodes - usage.parser_nodes);
    limits.max_owned_bytes = limits
        .max_owned_bytes
        .min(len)
        .min(session.max_parser_owned_bytes - usage.parser_owned_bytes);
    limits.max_operations = limits
        .max_operations
        .min(len)
        .min(session.max_parser_operations - usage.parser_operations);
    limits.lex.max_tokens = limits
        .lex
        .max_tokens
        .min(len)
        .min(session.max_parser_tokens - usage.parser_tokens);
    limits.lex.max_token_bytes = limits.lex.max_token_bytes.min(len);
    charge(
        &mut usage.parser_nodes,
        limits.max_nodes,
        session.max_parser_nodes,
        "parser nodes",
    )?;
    charge(
        &mut usage.parser_owned_bytes,
        limits.max_owned_bytes,
        session.max_parser_owned_bytes,
        "parser owned bytes",
    )?;
    charge(
        &mut usage.parser_operations,
        limits.max_operations,
        session.max_parser_operations,
        "parser operations",
    )?;
    charge(
        &mut usage.parser_tokens,
        limits.lex.max_tokens,
        session.max_parser_tokens,
        "parser tokens",
    )?;
    Ok(limits)
}
fn refund_parser(usage: &mut PageProgramSessionUsage, limits: ContentLimits, actual: ContentUsage) {
    usage.parser_nodes -= limits.max_nodes - actual.nodes;
    usage.parser_owned_bytes -= limits.max_owned_bytes - actual.owned_bytes;
    usage.parser_operations -= limits.max_operations - actual.operations;
    usage.parser_tokens -= limits.lex.max_tokens - actual.tokens;
}
#[derive(Default)]
struct Preflight {
    inside: bool,
    depth: usize,
    active_path: bool,
    pending_clip: bool,
    marked: usize,
}
impl Preflight {
    fn form_boundary(&self) -> Result<(), PageProgramError> {
        if self.inside || self.active_path || self.pending_clip {
            Err(error(PageProgramErrorKind::State(
                "Do with active text/path/clip",
            )))
        } else {
            Ok(())
        }
    }

    fn push(
        &mut self,
        op: &ValidatedOperation<'_>,
        max_depth: usize,
    ) -> Result<(), PageProgramError> {
        use OperatorKind::*;
        let state = |s| error(PageProgramErrorKind::State(s));
        match op.kind() {
            RenderingIntent if crate::program::relative_colorimetric(op) => {}
            Flatness if crate::program::default_flatness(op) => {}
            // Marked content does not paint. Optional content (BDC /OC) is
            // refused by the interpreter; edit admission refuses pages whose
            // property lists (ActualText, MCID) would need structure updates.
            MarkedPoint | MarkedPointProperties => {}
            BeginMarkedContent | BeginMarkedContentProperties => {
                self.marked = self
                    .marked
                    .checked_add(1)
                    .filter(|v| *v <= max_depth)
                    .ok_or_else(|| error(PageProgramErrorKind::LimitExceeded("marked depth")))?;
            }
            EndMarkedContent => {
                self.marked = self
                    .marked
                    .checked_sub(1)
                    .ok_or_else(|| state("EMC without BMC/BDC"))?;
            }
            Save | Restore | Concat => {
                if self.inside {
                    return Err(state("q/Q/cm inside BT unsupported"));
                }
                if op.kind() == Save {
                    self.depth = self
                        .depth
                        .checked_add(1)
                        .filter(|v| *v <= max_depth)
                        .ok_or_else(|| error(PageProgramErrorKind::LimitExceeded("save depth")))?;
                }
                if op.kind() == Restore {
                    self.depth = self
                        .depth
                        .checked_sub(1)
                        .ok_or_else(|| state("unbalanced Q"))?;
                }
            }
            BeginText => {
                if self.inside || self.active_path || self.pending_clip {
                    return Err(state("BT with active text/path/clip"));
                }
                self.inside = true;
            }
            EndText => {
                if !self.inside {
                    return Err(state("ET outside BT"));
                }
                self.inside = false;
            }
            MoveTo | LineTo | CurveTo | CurveToV | CurveToY | ClosePath | Rectangle => {
                if self.inside {
                    return Err(state("path inside BT unsupported"));
                }
                self.active_path = true;
            }
            Clip | EvenOddClip => {
                if self.inside {
                    return Err(state("clip inside BT unsupported"));
                }
                self.pending_clip = true;
            }
            Stroke
            | CloseStroke
            | Fill
            | EvenOddFill
            | FillStroke
            | EvenOddFillStroke
            | CloseFillStroke
            | CloseEvenOddFillStroke
            | EndPath => {
                if self.inside {
                    return Err(state("paint inside BT unsupported"));
                }
                self.active_path = false;
                self.pending_clip = false;
            }
            LineWidth | LineCap | LineJoin | MiterLimit | Dash | StrokeGray | FillGray
            | StrokeRgb | FillRgb | StrokeCmyk | FillCmyk => {}
            // The interpreter refuses unresolved spaces and pattern colors.
            StrokeColorSpace | FillColorSpace | StrokeColor | FillColor | StrokeColorPattern
            | FillColorPattern => {}
            Font | CharacterSpacing | WordSpacing | HorizontalScale | Leading | TextRise => {}
            TextRenderingMode => {
                if !matches!(op.operands()[0], ContentOperand::Integer(0 | 3)) {
                    return Err(error(PageProgramErrorKind::Unsupported(TextRenderingMode)));
                }
            }
            TextMove
            | TextMoveLeading
            | TextMatrix
            | TextNextLine
            | ShowText
            | ShowTextArray
            | NextLineShowText
            | SpacedNextLineShowText => {
                if !self.inside {
                    return Err(state("text position/show outside BT"));
                }
            }
            other => return Err(error(PageProgramErrorKind::Unsupported(other))),
        }
        Ok(())
    }
    fn finish(&self) -> Result<(), PageProgramError> {
        if self.inside
            || self.depth != 0
            || self.active_path
            || self.pending_clip
            || self.marked != 0
        {
            return Err(error(PageProgramErrorKind::State(
                "unfinished text, graphics stack, path, or marked content",
            )));
        }
        Ok(())
    }
}
