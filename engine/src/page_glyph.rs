// SPDX-License-Identifier: AGPL-3.0-or-later
//! Owned, unhinted embedded TrueType or explicitly pinned Base14 glyph placement.
//!
//! No rasterization or coverage is implied. Raw O0 glyf coordinates are normalized
//! horizontally by (hmtx.lsb - glyf.xMin), divided by unitsPerEm,
//! then transformed by T0's em-to-page matrix exactly once. PDF
//! widths determine T0 advance, never outline scaling. ToUnicode is not inverted.
//! Canonical pinned CFF coordinates retain their verified drawing origin and
//! normalize by their units per em, with no TrueType bearing correction.
//! Immutable canonical bytes form a fixed static partition; cached byte choices
//! and every occurrence/command/control point consume the runtime G1 ledgers.
//!
//! PP1, G0 loading/cache bytes, sfnt opening, cmap, and O0 have independent live
//! cumulative budget partitions. Orchestration buffers and Arc headers have a
//! separate full-request allocation ledger. Arc::new follows Rust's normal OOM
//! abort policy; its request is admitted before creation. Failed phase reservations
//! without a usage result remain charged. Cache hits do not repeat outline work.
use crate::builtin_outlines::{BuiltinOutline, BuiltinOutlineCommand};
use crate::document::Document;
pub use crate::embedded_glyph::PageOutlinePolicy;
use crate::embedded_glyph::{
    EmbeddedGlyphFont, GlyphChoice, GlyphError, GlyphFontLoader, GlyphLoadLimits, GlyphLoadUsage,
    GlyphRoute, LoadedBuiltinGlyphFont, LoadedGlyphFont,
};
use crate::font_resources::{FontIdentity, FontResourceUsage};
use crate::geometry::{GeometryError, PageSpace, Point};
use crate::object::ObjectId;
use crate::page_program::{
    PageProgram, PageProgramError, PageProgramLimits, PageProgramSession, PageProgramSessionLimits,
    PageProgramSessionUsage,
};
use crate::program::ProgramEventKind;
use crate::text_position::{CodePlacement, EmSpace, TextEvent};
use crate::truetype_cmap::{CmapLimits, CmapSelection, CmapUsage};
use crate::truetype_metrics::{HorizontalMetrics, MetricsError};
use crate::truetype_outline::{
    FontLimits, FontPoint2, FontUsage, GlyphId, GlyphOutline, OutlineCommand, OutlineLimits,
    OutlineUsage,
};
use std::{fmt, mem::size_of, ops::Range, sync::Arc};
#[derive(Clone, Copy, Debug)]
pub struct PageGlyphSessionLimits {
    pub outline_policy: PageOutlinePolicy,
    pub page: PageProgramSessionLimits,
    pub load: GlyphLoadLimits,
    pub font: FontLimits,
    pub cmap: CmapLimits,
    pub outline: OutlineLimits,
    pub max_cached_fonts: usize,
    pub max_cached_glyphs: usize,
    pub max_occurrences: usize,
    pub max_transformed_commands: usize,
    pub max_transform_points: usize,
    pub max_allocated_bytes: usize,
    pub max_metrics_work: usize,
    pub max_lookup_work: usize,
}
impl Default for PageGlyphSessionLimits {
    fn default() -> Self {
        Self {
            outline_policy: PageOutlinePolicy::EmbeddedOnly,
            page: PageProgramSessionLimits::default(),
            load: GlyphLoadLimits::default(),
            font: FontLimits {
                max_index_allocation_bytes: 64 << 20,
                max_work: 128 << 20,
                ..FontLimits::default()
            },
            cmap: CmapLimits {
                max_records: 32_768,
                max_segments: 1_048_576,
                max_work: 128 << 20,
                ..CmapLimits::default()
            },
            outline: OutlineLimits::default(),
            max_cached_fonts: 128,
            max_cached_glyphs: 32_768,
            max_occurrences: 1_000_000,
            max_transformed_commands: 16_000_000,
            max_transform_points: 32_000_000,
            max_allocated_bytes: 1 << 30,
            max_metrics_work: 1_000_000,
            max_lookup_work: 128_000_000,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct PageGlyphLimits {
    pub page: PageProgramLimits,
    pub font: FontLimits,
    pub cmap: CmapLimits,
    pub outline: OutlineLimits,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PageGlyphUsage {
    /// Includes slots conservatively reserved by failed cache preparation.
    pub cached_fonts: usize,
    pub cached_glyphs: usize,
    pub occurrences: usize,
    pub transformed_commands: usize,
    pub transform_points: usize,
    pub allocated_bytes: usize,
    pub metrics_work: usize,
    pub lookup_work: usize,
    pub font: FontUsage,
    pub cmap: CmapUsage,
    pub outline: OutlineUsage,
}
#[derive(Debug)]
pub enum PageGlyphErrorKind {
    Page(Box<PageProgramError>),
    Glyph(Box<GlyphError>),
    Geometry(GeometryError),
    Metrics(MetricsError),
    Unsupported(&'static str),
    Invariant(&'static str),
    LimitExceeded(&'static str),
    AllocationFailed,
}
#[derive(Debug)]
pub struct PageGlyphError {
    pub code_index: Option<usize>,
    pub operation_index: Option<usize>,
    pub kind: PageGlyphErrorKind,
}
impl fmt::Display for PageGlyphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "page glyph code {:?} operation {:?}: {:?}",
            self.code_index, self.operation_index, self.kind
        )
    }
}
impl std::error::Error for PageGlyphError {}
fn error(kind: PageGlyphErrorKind) -> PageGlyphError {
    PageGlyphError {
        code_index: None,
        operation_index: None,
        kind,
    }
}
fn glyph_error(e: GlyphError) -> PageGlyphError {
    error(PageGlyphErrorKind::Glyph(Box::new(e)))
}
fn at(mut e: PageGlyphError, index: usize, code: &CodePlacement) -> PageGlyphError {
    e.code_index = Some(index);
    e.operation_index = Some(code.operation_index());
    e
}
fn charge(
    value: &mut usize,
    n: usize,
    limit: usize,
    label: &'static str,
) -> Result<(), PageGlyphError> {
    *value = value
        .checked_add(n)
        .filter(|v| *v <= limit)
        .ok_or_else(|| error(PageGlyphErrorKind::LimitExceeded(label)))?;
    Ok(())
}
fn bytes(count: usize, size: usize) -> Result<usize, PageGlyphError> {
    count
        .checked_mul(size)
        .ok_or_else(|| error(PageGlyphErrorKind::LimitExceeded("allocation arithmetic")))
}
fn reserve<T>(
    values: &mut Vec<T>,
    additional: usize,
    max: usize,
    usage: &mut PageGlyphUsage,
    limit: usize,
) -> Result<(), PageGlyphError> {
    let end = values
        .len()
        .checked_add(additional)
        .filter(|v| *v <= max)
        .ok_or_else(|| error(PageGlyphErrorKind::LimitExceeded("arena entries")))?;
    if end <= values.capacity() {
        return Ok(());
    }
    let capacity = values.capacity().saturating_mul(2).min(max).max(end);
    charge(
        &mut usage.allocated_bytes,
        bytes(capacity, size_of::<T>())?,
        limit,
        "orchestration allocation",
    )?;
    values
        .try_reserve_exact(capacity - values.len())
        .map_err(|_| error(PageGlyphErrorKind::AllocationFailed))
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageGlyphCommand {
    MoveTo(Point<PageSpace>),
    LineTo(Point<PageSpace>),
    QuadTo {
        control: Point<PageSpace>,
        end: Point<PageSpace>,
    },
    CubicTo {
        control1: Point<PageSpace>,
        control2: Point<PageSpace>,
        end: Point<PageSpace>,
    },
    Close,
}
#[derive(Debug)]
pub enum PageGlyphSource {
    EmbeddedTrueType(EmbeddedTrueTypeSource),
    PinnedBase14(&'static BuiltinOutline),
}
#[derive(Debug)]
pub struct EmbeddedTrueTypeSource {
    program_id: ObjectId,
    /// Simple fonts: the cmap route. None for CIDFontType2 (CID→GID).
    choice: Option<GlyphChoice>,
    selection: Option<CmapSelection>,
    glyph_id: u16,
    outline: Arc<GlyphOutline>,
    metric_lsb: i16,
    origin_shift: i32,
    hhea: Range<usize>,
    hmtx: Range<usize>,
}
#[derive(Debug)]
pub struct PageGlyph {
    code_index: usize,
    event_index: usize,
    context_id: usize,
    font_id: ObjectId,
    source: PageGlyphSource,
    units_per_em: u16,
    command_range: Range<usize>,
}
impl PageGlyph {
    pub fn code_index(&self) -> usize {
        self.code_index
    }
    pub fn event_index(&self) -> usize {
        self.event_index
    }
    pub fn context_id(&self) -> usize {
        self.context_id
    }
    pub fn font_id(&self) -> ObjectId {
        self.font_id
    }
    pub fn source(&self) -> &PageGlyphSource {
        &self.source
    }
    pub fn embedded_true_type(&self) -> Option<&EmbeddedTrueTypeSource> {
        match &self.source {
            PageGlyphSource::EmbeddedTrueType(source) => Some(source),
            _ => None,
        }
    }
    pub fn pinned_base14(&self) -> Option<&'static BuiltinOutline> {
        match &self.source {
            PageGlyphSource::PinnedBase14(source) => Some(*source),
            _ => None,
        }
    }
    pub fn units_per_em(&self) -> u16 {
        self.units_per_em
    }
    pub fn command_range(&self) -> Range<usize> {
        self.command_range.clone()
    }
}
impl EmbeddedTrueTypeSource {
    pub fn program_id(&self) -> ObjectId {
        self.program_id
    }
    pub fn choice(&self) -> Option<GlyphChoice> {
        self.choice
    }
    pub fn selection(&self) -> Option<&CmapSelection> {
        self.selection.as_ref()
    }
    pub fn glyph_id(&self) -> u16 {
        self.glyph_id
    }
    pub fn outline(&self) -> &GlyphOutline {
        &self.outline
    }
    pub fn metric_lsb(&self) -> i16 {
        self.metric_lsb
    }
    pub fn origin_shift(&self) -> i32 {
        self.origin_shift
    }
    pub fn hhea_range(&self) -> Range<usize> {
        self.hhea.clone()
    }
    pub fn hmtx_range(&self) -> Range<usize> {
        self.hmtx.clone()
    }
}
#[derive(Debug)]
pub struct PageGlyphProgram {
    page: PageProgram,
    glyphs: Vec<PageGlyph>,
    commands: Vec<PageGlyphCommand>,
}
impl PageGlyphProgram {
    pub fn page_program(&self) -> &PageProgram {
        &self.page
    }
    pub fn glyphs(&self) -> &[PageGlyph] {
        &self.glyphs
    }
    pub fn commands(&self) -> &[PageGlyphCommand] {
        &self.commands
    }
}
struct CachedOutline {
    outline: Arc<GlyphOutline>,
    lsb: i16,
    shift: i32,
}
struct CachedFont {
    source: EmbeddedGlyphFont,
    units: u16,
    /// Simple TrueType cmap route; None selects by CID through the source.
    map: Option<(CmapSelection, [GlyphChoice; 256])>,
    outlines: Vec<CachedOutline>,
    hhea: Range<usize>,
    hmtx: Range<usize>,
}
impl CachedFont {
    fn glyph(&self, code: u32) -> Result<(GlyphId, Option<GlyphChoice>), GlyphError> {
        let Some((_, choices)) = &self.map else {
            let gid = self
                .source
                .cid_glyph(code)
                .ok_or(GlyphError::Missing(code))?;
            return Ok((GlyphId::from_raw(gid), None));
        };
        let choice = *usize::try_from(code)
            .ok()
            .and_then(|i| choices.get(i))
            .ok_or(GlyphError::UnsupportedCode(code))?;
        if choice.route() == GlyphRoute::UnsupportedNameFallback {
            return Err(GlyphError::UnsupportedCode(code));
        }
        let gid = choice.glyph_id().ok_or(GlyphError::Missing(code))?;
        Ok((gid, Some(choice)))
    }
}
struct CachedBuiltinFont {
    source: LoadedBuiltinGlyphFont,
    choices: [Option<&'static BuiltinOutline>; 256],
}
pub struct PageGlyphSession<'doc, 'input> {
    page: PageProgramSession<'doc, 'input>,
    loader: GlyphFontLoader<'doc, 'input>,
    cache: Vec<CachedFont>,
    builtin_cache: Vec<CachedBuiltinFont>,
    limits: PageGlyphSessionLimits,
    usage: PageGlyphUsage,
}
impl<'doc, 'input> PageGlyphSession<'doc, 'input> {
    pub fn new(
        document: &'doc Document<'input>,
        limits: PageGlyphSessionLimits,
    ) -> Result<Self, PageGlyphError> {
        let page = PageProgramSession::new(document, limits.page)
            .map_err(|e| error(PageGlyphErrorKind::Page(Box::new(e))))?;
        Ok(Self {
            page,
            loader: GlyphFontLoader::new(document, limits.load),
            cache: Vec::new(),
            builtin_cache: Vec::new(),
            limits,
            usage: PageGlyphUsage::default(),
        })
    }
    pub fn page_count(&self) -> usize {
        self.page.page_count()
    }
    pub fn usage(&self) -> PageGlyphUsage {
        self.usage
    }
    pub fn page_usage(&self) -> PageProgramSessionUsage {
        self.page.usage()
    }
    pub fn font_resource_usage(&self) -> FontResourceUsage {
        self.page.font_usage()
    }
    pub fn load_usage(&self) -> GlyphLoadUsage {
        self.loader.usage()
    }
    pub fn trace_page(
        &mut self,
        index: usize,
        limits: PageGlyphLimits,
    ) -> Result<PageGlyphProgram, PageGlyphError> {
        self.trace_page_with_annotations(index, limits, &[])
    }
    pub fn trace_page_with_annotations(
        &mut self,
        index: usize,
        limits: PageGlyphLimits,
        annotations: &[crate::page_program::AnnotationAppearance],
    ) -> Result<PageGlyphProgram, PageGlyphError> {
        let page = self
            .page
            .trace_page_with_annotations(index, limits.page, annotations)
            .map_err(|e| {
                let operation_index = e.operation_index;
                PageGlyphError {
                    code_index: None,
                    operation_index,
                    kind: PageGlyphErrorKind::Page(Box::new(e)),
                }
            })?;
        let codes = page.program().text().codes();
        charge(
            &mut self.usage.occurrences,
            codes.len(),
            self.limits.max_occurrences,
            "glyph occurrences",
        )?;
        // Prepare only fonts of actual code events; PP1 has already validated
        // every Tf, including selections that never show text.
        for (code_index, code) in codes.iter().enumerate() {
            let font_id = font_id(&page, code).map_err(|e| at(e, code_index, code))?;
            lookup_charge(&mut self.usage, self.limits.max_lookup_work)?;
            if find_font(
                &self.cache,
                font_id,
                &mut self.usage,
                self.limits.max_lookup_work,
            )?
            .is_some()
                || find_builtin_font(
                    &self.builtin_cache,
                    font_id,
                    &mut self.usage,
                    self.limits.max_lookup_work,
                )?
                .is_some()
            {
                continue;
            }
            charge(
                &mut self.usage.cached_fonts,
                1,
                self.limits.max_cached_fonts,
                "cached fonts",
            )?;
            let source = self
                .loader
                .load_source(font_id, self.limits.outline_policy)
                .map_err(|e| at(glyph_error(e), code_index, code))?;
            let source = match source {
                LoadedGlyphFont::EmbeddedTrueType(source) => source,
                LoadedGlyphFont::PinnedBase14(source) => {
                    charge(
                        &mut self.usage.lookup_work,
                        256,
                        self.limits.max_lookup_work,
                        "builtin cache initialization",
                    )?;
                    reserve(
                        &mut self.builtin_cache,
                        1,
                        self.limits.max_cached_fonts,
                        &mut self.usage,
                        self.limits.max_allocated_bytes,
                    )?;
                    self.builtin_cache.push(CachedBuiltinFont {
                        source,
                        choices: [None; 256],
                    });
                    continue;
                }
            };
            reserve(
                &mut self.cache,
                1,
                self.limits.max_cached_fonts,
                &mut self.usage,
                self.limits.max_allocated_bytes,
            )?;
            let font_limits = reserve_font(&mut self.usage, self.limits.font, limits.font)?;
            let (units, map) = if source.cid_to_gid().is_some() {
                let font = source
                    .open(font_limits)
                    .map_err(|e| at(glyph_error(e), code_index, code))?;
                refund_font(&mut self.usage, font_limits, font.usage());
                (font.units_per_em(), None)
            } else {
                let cmap_limits = reserve_cmap(&mut self.usage, self.limits.cmap, limits.cmap)?;
                let prepared = source
                    .outlines(font_limits, cmap_limits)
                    .map_err(|e| at(glyph_error(e), code_index, code))?;
                refund_font(&mut self.usage, font_limits, prepared.font_usage());
                refund_cmap(&mut self.usage, cmap_limits, prepared.map().usage());
                let map = (
                    prepared.map().selection().clone(),
                    *prepared.map().choices(),
                );
                (prepared.units_per_em(), Some(map))
            };
            self.cache.push(CachedFont {
                source,
                units,
                map,
                outlines: Vec::new(),
                hhea: 0..0,
                hmtx: 0..0,
            });
        }
        // Decode each missing GID once per font, with one font open per batch.
        // Reopens are charged even though their index storage is temporary.
        for cache_index in 0..self.cache.len() {
            charge(
                &mut self.usage.lookup_work,
                512,
                self.limits.max_lookup_work,
                "cache batch work",
            )?;
            let cached = &mut self.cache[cache_index];
            // First occurrence of each missing GID, in code order.
            let mut needed = std::collections::BTreeMap::new();
            for (code_index, code) in codes.iter().enumerate() {
                lookup_charge(&mut self.usage, self.limits.max_lookup_work)?;
                if font_id(&page, code).map_err(|e| at(e, code_index, code))?
                    != cached.source.font_id()
                {
                    continue;
                }
                let (gid, _) = cached
                    .glyph(code.code())
                    .map_err(|e| at(glyph_error(e), code_index, code))?;
                if find_outline(
                    &cached.outlines,
                    gid,
                    &mut self.usage,
                    self.limits.max_lookup_work,
                )?
                .is_none()
                {
                    needed.entry(gid.get()).or_insert(code_index);
                }
            }
            if needed.is_empty() {
                continue;
            }
            let first_code_index = *needed.values().min().expect("nonempty glyph batch");
            let first_code = &codes[first_code_index];
            let font_limits = reserve_font(&mut self.usage, self.limits.font, limits.font)?;
            let font = cached
                .source
                .open(font_limits)
                .map_err(|e| at(glyph_error(e), first_code_index, first_code))?;
            refund_font(&mut self.usage, font_limits, font.usage());
            charge(
                &mut self.usage.metrics_work,
                4 * font.table_records().len() + 40,
                self.limits.max_metrics_work,
                "metrics validation work",
            )?;
            let metrics = HorizontalMetrics::open(&font).map_err(|e| {
                at(
                    error(PageGlyphErrorKind::Metrics(e)),
                    first_code_index,
                    first_code,
                )
            })?;
            cached.hhea = metrics.hhea_range();
            cached.hmtx = metrics.hmtx_range();
            let outline_limits =
                remaining_outline(self.usage.outline, self.limits.outline, limits.outline);
            let mut decoder = font.session(outline_limits);
            let result = (|| {
                for (gid, code_index) in needed {
                    let code = &codes[code_index];
                    let gid = GlyphId::from_raw(gid);
                    if find_outline(
                        &cached.outlines,
                        gid,
                        &mut self.usage,
                        self.limits.max_lookup_work,
                    )?
                    .is_some()
                    {
                        continue;
                    }
                    charge(
                        &mut self.usage.cached_glyphs,
                        1,
                        self.limits.max_cached_glyphs,
                        "cached glyphs",
                    )?;
                    reserve(
                        &mut cached.outlines,
                        1,
                        self.limits.max_cached_glyphs,
                        &mut self.usage,
                        self.limits.max_allocated_bytes,
                    )?;
                    // Arc header + payload request; O0 owns its separately
                    // budgeted command/point allocations, shared unchanged.
                    charge(
                        &mut self.usage.allocated_bytes,
                        size_of::<GlyphOutline>() + 2 * size_of::<usize>(),
                        self.limits.max_allocated_bytes,
                        "shared outline header",
                    )?;
                    let checked = font
                        .glyph_id(u32::from(gid.get()))
                        .map_err(|e| at(glyph_error(GlyphError::Font(e)), code_index, code))?;
                    let outline = decoder
                        .decode(checked)
                        .map_err(|e| at(glyph_error(GlyphError::Font(e)), code_index, code))?;
                    charge(
                        &mut self.usage.metrics_work,
                        2,
                        self.limits.max_metrics_work,
                        "metrics query work",
                    )?;
                    let lsb = metrics
                        .get(checked)
                        .map_err(|e| at(error(PageGlyphErrorKind::Metrics(e)), code_index, code))?
                        .left_side_bearing();
                    let shift = i32::from(lsb) - outline.header_bounds().map_or(0, |b| b.x_min());
                    cached.outlines.push(CachedOutline {
                        outline: Arc::new(outline),
                        lsb,
                        shift,
                    });
                }
                Ok(())
            })();
            add_outline(&mut self.usage.outline, decoder.usage());
            result?;
        }
        // Canonical data is immutable: retain only byte-to-record choices. Each
        // first choice and every failed retry pays the bounded exact-name lookup.
        for (code_index, code) in codes.iter().enumerate() {
            let font_id = font_id(&page, code).map_err(|e| at(e, code_index, code))?;
            let Some(index) = find_builtin_font(
                &self.builtin_cache,
                font_id,
                &mut self.usage,
                self.limits.max_lookup_work,
            )?
            else {
                continue;
            };
            let cached = &mut self.builtin_cache[index];
            let byte = u8::try_from(code.code())
                .map_err(|_| error(PageGlyphErrorKind::Invariant("one-byte builtin code")))?;
            let slot = usize::from(byte);
            if cached.choices[slot].is_some() {
                continue;
            }
            let face = cached.source.face();
            charge(
                &mut self.usage.lookup_work,
                BuiltinOutline::lookup_work_bound(face),
                self.limits.max_lookup_work,
                "canonical glyph lookup",
            )?;
            let name = cached
                .source
                .glyph_name(byte)
                .filter(|name| *name != b".notdef")
                .ok_or_else(|| {
                    at(
                        glyph_error(GlyphError::Missing(code.code())),
                        code_index,
                        code,
                    )
                })?;
            let outline = BuiltinOutline::lookup(face, name).ok_or_else(|| {
                at(
                    glyph_error(GlyphError::UnsupportedCode(code.code())),
                    code_index,
                    code,
                )
            })?;
            charge(
                &mut self.usage.cached_glyphs,
                1,
                self.limits.max_cached_glyphs,
                "cached builtin choices",
            )?;
            cached.choices[slot] = Some(outline);
        }
        let mut glyphs = Vec::new();
        let mut commands = Vec::new();
        reserve(
            &mut glyphs,
            codes.len(),
            self.limits.max_occurrences,
            &mut self.usage,
            self.limits.max_allocated_bytes,
        )?;
        for (event_index, event) in page.program().events().iter().enumerate() {
            lookup_charge(&mut self.usage, self.limits.max_lookup_work)?;
            let ProgramEventKind::Text {
                event: text_event,
                context,
            } = event.kind()
            else {
                continue;
            };
            let TextEvent::Code(code_index) = page.program().text().events()[text_event] else {
                continue;
            };
            let code = &codes[code_index];
            let font_id = font_id(&page, code).map_err(|e| at(e, code_index, code))?;
            if let Some(index) = find_builtin_font(
                &self.builtin_cache,
                font_id,
                &mut self.usage,
                self.limits.max_lookup_work,
            )? {
                let outline = usize::try_from(code.code())
                    .ok()
                    .and_then(|slot| self.builtin_cache[index].choices.get(slot).copied())
                    .flatten()
                    .ok_or_else(|| {
                        error(PageGlyphErrorKind::Invariant("prepared builtin glyph"))
                    })?;
                let range =
                    place_builtin(outline, code, &mut commands, &mut self.usage, self.limits)
                        .map_err(|e| at(e, code_index, code))?;
                glyphs.push(PageGlyph {
                    code_index,
                    event_index,
                    context_id: context,
                    font_id,
                    source: PageGlyphSource::PinnedBase14(outline),
                    units_per_em: outline.units_per_em(),
                    command_range: range,
                });
                continue;
            }
            let cache_index = find_font(
                &self.cache,
                font_id,
                &mut self.usage,
                self.limits.max_lookup_work,
            )?
            .ok_or_else(|| error(PageGlyphErrorKind::Invariant("prepared glyph font")))?;
            let cached = &self.cache[cache_index];
            let (gid, choice) = cached
                .glyph(code.code())
                .map_err(|_| error(PageGlyphErrorKind::Invariant("prepared glyph")))?;
            let outline_index = find_outline(
                &cached.outlines,
                gid,
                &mut self.usage,
                self.limits.max_lookup_work,
            )?
            .ok_or_else(|| error(PageGlyphErrorKind::Invariant("cached glyph")))?;
            let outline = &cached.outlines[outline_index];
            charge(
                &mut self.usage.transformed_commands,
                outline.outline.commands().len(),
                self.limits.max_transformed_commands,
                "transformed commands",
            )?;
            let points = outline
                .outline
                .commands()
                .iter()
                .map(|c| match c {
                    OutlineCommand::MoveTo(_) | OutlineCommand::LineTo(_) => 1,
                    OutlineCommand::QuadTo { .. } => 2,
                    OutlineCommand::Close => 0,
                })
                .sum();
            charge(
                &mut self.usage.transform_points,
                points,
                self.limits.max_transform_points,
                "transform points",
            )?;
            reserve(
                &mut commands,
                outline.outline.commands().len(),
                self.limits.max_transformed_commands,
                &mut self.usage,
                self.limits.max_allocated_bytes,
            )?;
            let start = commands.len();
            for command in outline.outline.commands() {
                let placed = match *command {
                    OutlineCommand::MoveTo(p) => PageGlyphCommand::MoveTo(
                        place(p, cached.units, outline.shift, code)
                            .map_err(|e| at(e, code_index, code))?,
                    ),
                    OutlineCommand::LineTo(p) => PageGlyphCommand::LineTo(
                        place(p, cached.units, outline.shift, code)
                            .map_err(|e| at(e, code_index, code))?,
                    ),
                    OutlineCommand::QuadTo { control, end } => PageGlyphCommand::QuadTo {
                        control: place(control, cached.units, outline.shift, code)
                            .map_err(|e| at(e, code_index, code))?,
                        end: place(end, cached.units, outline.shift, code)
                            .map_err(|e| at(e, code_index, code))?,
                    },
                    OutlineCommand::Close => PageGlyphCommand::Close,
                };
                commands.push(placed);
            }
            glyphs.push(PageGlyph {
                code_index,
                event_index,
                context_id: context,
                font_id,
                source: PageGlyphSource::EmbeddedTrueType(EmbeddedTrueTypeSource {
                    program_id: cached.source.program_id(),
                    choice,
                    selection: cached.map.as_ref().map(|(selection, _)| selection.clone()),
                    glyph_id: gid.get(),
                    outline: Arc::clone(&outline.outline),
                    metric_lsb: outline.lsb,
                    origin_shift: outline.shift,
                    hhea: cached.hhea.clone(),
                    hmtx: cached.hmtx.clone(),
                }),
                units_per_em: cached.units,
                command_range: start..commands.len(),
            });
        }
        if glyphs.len() != codes.len() {
            return Err(error(PageGlyphErrorKind::Invariant(
                "code/event attachment count",
            )));
        }
        Ok(PageGlyphProgram {
            page,
            glyphs,
            commands,
        })
    }
}
fn font_id(page: &PageProgram, code: &CodePlacement) -> Result<ObjectId, PageGlyphError> {
    match page
        .fonts()
        .get(code.font_id().index())
        .ok_or_else(|| error(PageGlyphErrorKind::Invariant("PP1 font registry ID")))?
        .identity()
    {
        FontIdentity::Indirect(id) => Ok(id),
        FontIdentity::Direct { .. } => Err(error(PageGlyphErrorKind::Unsupported(
            "direct font dictionary outline loading",
        ))),
    }
}
fn place(
    point: FontPoint2,
    units: u16,
    origin_shift: i32,
    code: &CodePlacement,
) -> Result<Point<PageSpace>, PageGlyphError> {
    let denominator = 2. * f64::from(units);
    let point = Point::<EmSpace>::new(
        (f64::from(point.x2()) + 2. * f64::from(origin_shift)) / denominator,
        f64::from(point.y2()) / denominator,
    )
    .map_err(|e| error(PageGlyphErrorKind::Geometry(e)))?;
    code.transform()
        .transform_point(point)
        .map_err(|e| error(PageGlyphErrorKind::Geometry(e)))
}
fn reserve_font(
    usage: &mut PageGlyphUsage,
    total: FontLimits,
    mut cap: FontLimits,
) -> Result<FontLimits, PageGlyphError> {
    cap.max_input_bytes = cap.max_input_bytes.min(total.max_input_bytes);
    cap.max_tables = cap.max_tables.min(total.max_tables);
    cap.max_glyphs = cap.max_glyphs.min(total.max_glyphs);
    cap.max_index_allocation_bytes = cap
        .max_index_allocation_bytes
        .min(total.max_index_allocation_bytes - usage.font.index_allocation_bytes);
    cap.max_work = cap.max_work.min(total.max_work - usage.font.work);
    usage.font.index_allocation_bytes += cap.max_index_allocation_bytes;
    usage.font.work += cap.max_work;
    Ok(cap)
}
fn refund_font(usage: &mut PageGlyphUsage, reserved: FontLimits, actual: FontUsage) {
    usage.font.index_allocation_bytes -=
        reserved.max_index_allocation_bytes - actual.index_allocation_bytes;
    usage.font.work -= reserved.max_work - actual.work;
}
fn reserve_cmap(
    usage: &mut PageGlyphUsage,
    total: CmapLimits,
    mut cap: CmapLimits,
) -> Result<CmapLimits, PageGlyphError> {
    macro_rules! field {
        ($max:ident,$used:ident) => {
            cap.$max = cap.$max.min(total.$max - usage.cmap.$used);
            usage.cmap.$used += cap.$max;
        };
    }
    field!(max_records, directory_records);
    field!(max_selection_attempts, selection_attempts);
    field!(max_segments, segments);
    field!(max_validation_codes, validated_codes);
    field!(max_queries, queries);
    field!(max_work, work);
    Ok(cap)
}
fn refund_cmap(usage: &mut PageGlyphUsage, reserved: CmapLimits, actual: CmapUsage) {
    macro_rules! field {
        ($max:ident,$used:ident) => {
            usage.cmap.$used -= reserved.$max - actual.$used;
        };
    }
    field!(max_records, directory_records);
    field!(max_selection_attempts, selection_attempts);
    field!(max_segments, segments);
    field!(max_validation_codes, validated_codes);
    field!(max_queries, queries);
    field!(max_work, work);
}
fn remaining_outline(
    used: OutlineUsage,
    total: OutlineLimits,
    mut cap: OutlineLimits,
) -> OutlineLimits {
    macro_rules! field {
        ($max:ident,$used:ident) => {
            cap.$max = cap.$max.min(total.$max - used.$used);
        };
    }
    field!(max_decode_attempts, decode_attempts);
    field!(max_total_glyph_bytes, glyph_bytes);
    field!(max_total_contours, contours);
    field!(max_total_points, points);
    field!(max_total_commands, commands);
    field!(max_allocation_bytes, allocation_bytes);
    field!(max_work, work);
    cap.max_contours_per_glyph = cap.max_contours_per_glyph.min(total.max_contours_per_glyph);
    cap.max_points_per_glyph = cap.max_points_per_glyph.min(total.max_points_per_glyph);
    cap.max_instruction_bytes_per_glyph = cap
        .max_instruction_bytes_per_glyph
        .min(total.max_instruction_bytes_per_glyph);
    cap
}
fn add_outline(total: &mut OutlineUsage, used: OutlineUsage) {
    total.decode_attempts += used.decode_attempts;
    total.glyph_bytes += used.glyph_bytes;
    total.contours += used.contours;
    total.points += used.points;
    total.commands += used.commands;
    total.allocation_bytes += used.allocation_bytes;
    total.work += used.work;
}

fn lookup_charge(usage: &mut PageGlyphUsage, limit: usize) -> Result<(), PageGlyphError> {
    charge(
        &mut usage.lookup_work,
        1,
        limit,
        "orchestration lookup work",
    )
}
fn find_font(
    cache: &[CachedFont],
    id: ObjectId,
    usage: &mut PageGlyphUsage,
    limit: usize,
) -> Result<Option<usize>, PageGlyphError> {
    for (index, font) in cache.iter().enumerate() {
        lookup_charge(usage, limit)?;
        if font.source.font_id() == id {
            return Ok(Some(index));
        }
    }
    Ok(None)
}
fn find_outline(
    cache: &[CachedOutline],
    id: crate::truetype_outline::GlyphId,
    usage: &mut PageGlyphUsage,
    limit: usize,
) -> Result<Option<usize>, PageGlyphError> {
    for (index, outline) in cache.iter().enumerate() {
        lookup_charge(usage, limit)?;
        if outline.outline.id() == id {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn find_builtin_font(
    cache: &[CachedBuiltinFont],
    id: ObjectId,
    usage: &mut PageGlyphUsage,
    limit: usize,
) -> Result<Option<usize>, PageGlyphError> {
    for (index, font) in cache.iter().enumerate() {
        lookup_charge(usage, limit)?;
        if font.source.font_id() == id {
            return Ok(Some(index));
        }
    }
    Ok(None)
}
fn place_builtin(
    outline: &'static BuiltinOutline,
    code: &CodePlacement,
    commands: &mut Vec<PageGlyphCommand>,
    usage: &mut PageGlyphUsage,
    limits: PageGlyphSessionLimits,
) -> Result<Range<usize>, PageGlyphError> {
    let raw = outline.commands();
    charge(
        &mut usage.transformed_commands,
        raw.len(),
        limits.max_transformed_commands,
        "transformed commands",
    )?;
    // The command cap is admitted before either traversal of immutable data.
    let points = raw
        .iter()
        .map(|c| match c {
            BuiltinOutlineCommand::MoveTo { .. } | BuiltinOutlineCommand::LineTo { .. } => 1,
            BuiltinOutlineCommand::CubicTo { .. } => 3,
            BuiltinOutlineCommand::Close => 0,
        })
        .sum();
    charge(
        &mut usage.transform_points,
        points,
        limits.max_transform_points,
        "transform points",
    )?;
    reserve(
        commands,
        raw.len(),
        limits.max_transformed_commands,
        usage,
        limits.max_allocated_bytes,
    )?;
    let place = |x: i32, y: i32| -> Result<Point<PageSpace>, PageGlyphError> {
        let em = Point::<EmSpace>::new(
            f64::from(x) / f64::from(outline.units_per_em()),
            f64::from(y) / f64::from(outline.units_per_em()),
        )
        .map_err(|e| error(PageGlyphErrorKind::Geometry(e)))?;
        code.transform()
            .transform_point(em)
            .map_err(|e| error(PageGlyphErrorKind::Geometry(e)))
    };
    let start = commands.len();
    for command in raw {
        commands.push(match *command {
            BuiltinOutlineCommand::MoveTo { x, y } => PageGlyphCommand::MoveTo(place(x, y)?),
            BuiltinOutlineCommand::LineTo { x, y } => PageGlyphCommand::LineTo(place(x, y)?),
            BuiltinOutlineCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => PageGlyphCommand::CubicTo {
                control1: place(x1, y1)?,
                control2: place(x2, y2)?,
                end: place(x, y)?,
            },
            BuiltinOutlineCommand::Close => PageGlyphCommand::Close,
        });
    }
    Ok(start..commands.len())
}
