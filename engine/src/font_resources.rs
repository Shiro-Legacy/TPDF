// SPDX-License-Identifier: AGPL-3.0-or-later
// Resource inheritance and font declarations reference MuPDF 1.28.1
// source/pdf/pdf-object.c, pdf-font.c and pdf-unicode.c. Artifex; see NOTICE.
//! Lazy, document-bound simple-font Unicode and explicit advance declarations.
//! No font-program glyph selection or font-program metric loading.
//! Resolved null means absent, including resource inheritance. Resources replace
//! the whole ancestor dictionary. Strict ToUnicode is authoritative. Successful
//! maps/fonts are shared; attempted work remains charged after failures.
use crate::font_metrics::{CidWidthRange, FontAdvance, HorizontalMetrics, MetricsError};
use crate::{
    cmap::{CMapError, CMapLimits, CodeWidth, ToUnicodeMap, parse_to_unicode},
    document::{Document, DocumentError},
    object::{Object, ObjectId},
    pages::{PageError, PageLimits, collect_pages},
    simple_encoding::{
        Difference, EncodingError, EncodingErrorKind, EncodingLimits, NamedEncoding, SimpleEncoding,
    },
    stream::StreamLimits,
    stream_object::{StreamObjectError, decode_document_stream_with_usage},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    rc::Rc,
};

use crate::builtin_metrics::{BuiltinFace, BuiltinMetrics};

type Dictionary = BTreeMap<Vec<u8>, Object>;
#[derive(Debug, Clone, Copy)]
pub struct FontResourceLimits {
    pub pages: PageLimits,
    pub max_reference_depth: usize,
    pub max_parent_depth: usize,
    pub max_calls: usize,
    pub max_reference_hops: usize,
    pub max_metadata_items: usize,
    pub max_bindings: usize,
    pub max_fonts: usize,
    pub max_metric_attempts: usize,
    pub max_metrics: usize,
    pub max_builtin_metric_attempts: usize,
    pub max_builtin_metrics: usize,
    pub max_builtin_slots: usize,
    pub max_retained_builtin_slots: usize,
    pub max_width_entries: usize,
    pub max_retained_width_entries: usize,
    pub max_maps: usize,
    pub max_name_bytes: usize,
    pub max_encoded_bytes: usize,
    pub max_decoded_work_bytes: usize,
    pub max_cmap_input_bytes: usize,
    pub max_cmap_entries: usize,
    pub max_cmap_payload_bytes: usize,
    pub max_difference_items: usize,
    pub max_difference_name_bytes: usize,
    pub stream: StreamLimits,
    pub cmap: CMapLimits,
    pub encoding: EncodingLimits,
}
impl Default for FontResourceLimits {
    fn default() -> Self {
        Self {
            pages: PageLimits::default(),
            max_reference_depth: 64,
            max_parent_depth: 64,
            max_calls: 100_000,
            max_reference_hops: 1_000_000,
            max_metadata_items: 100_000,
            max_bindings: 100_000,
            max_fonts: 4096,
            max_metric_attempts: 100_000,
            max_metrics: 4096,
            max_builtin_metric_attempts: 100_000,
            max_builtin_metrics: 4096,
            max_builtin_slots: 1_048_576,
            max_retained_builtin_slots: 1_048_576,
            max_width_entries: 1_048_576,
            max_retained_width_entries: 1_048_576,
            max_maps: 1024,
            max_name_bytes: 4 * 1024 * 1024,
            max_encoded_bytes: 64 * 1024 * 1024,
            max_decoded_work_bytes: 128 * 1024 * 1024,
            max_cmap_input_bytes: 64 * 1024 * 1024,
            max_cmap_entries: 1_048_576,
            max_cmap_payload_bytes: 64 * 1024 * 1024,
            max_difference_items: 100_000,
            max_difference_name_bytes: 16 * 1024 * 1024,
            stream: StreamLimits::default(),
            cmap: CMapLimits::default(),
            encoding: EncodingLimits::default(),
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FontResourceUsage {
    pub calls: usize,
    pub reference_hops: usize,
    pub metadata_items: usize,
    pub bindings: usize,
    pub fonts: usize,
    pub metric_attempts: usize,
    pub metrics: usize,
    pub builtin_metric_attempts: usize,
    pub builtin_metrics: usize,
    pub builtin_slots: usize,
    pub retained_builtin_slots: usize,
    pub width_entries: usize,
    pub retained_width_entries: usize,
    pub maps: usize,
    pub name_bytes: usize,
    pub encoded_bytes: usize,
    pub decoded_work_bytes: usize,
    pub cmap_input_bytes: usize,
    pub cmap_entries: usize,
    pub cmap_payload_bytes: usize,
    pub difference_items: usize,
    pub difference_name_bytes: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FontIdentity {
    Indirect(ObjectId),
    Direct {
        resource_owner: ObjectId,
        resource_index: usize,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimpleFontSubtype {
    Type1,
    TrueType,
    /// Composite font with Identity-H encoding and one CIDFontType2 descendant.
    Type0,
}
#[derive(Debug)]
pub enum SimpleFontMapping {
    ToUnicode(Rc<ToUnicodeMap>),
    ExplicitEncoding {
        base: NamedEncoding,
        encoding: Box<SimpleEncoding>,
    },
}
#[derive(Debug)]
pub struct LoadedSimpleFont {
    identity: FontIdentity,
    subtype: SimpleFontSubtype,
    base_font: Option<Vec<u8>>,
    mapping: SimpleFontMapping,
}
impl LoadedSimpleFont {
    pub fn identity(&self) -> FontIdentity {
        self.identity
    }
    pub fn subtype(&self) -> SimpleFontSubtype {
        self.subtype
    }
    pub fn base_font(&self) -> Option<&[u8]> {
        self.base_font.as_deref()
    }
    pub fn mapping(&self) -> &SimpleFontMapping {
        &self.mapping
    }
}
#[derive(Debug)]
pub struct LoadedFontMetrics {
    identity: FontIdentity,
    metrics: HorizontalMetrics,
}
impl LoadedFontMetrics {
    pub fn identity(&self) -> FontIdentity {
        self.identity
    }
    pub fn metrics(&self) -> &HorizontalMetrics {
        &self.metrics
    }
}
#[derive(Debug)]
pub struct LoadedBuiltinMetrics {
    identity: FontIdentity,
    metrics: BuiltinMetrics,
}
impl LoadedBuiltinMetrics {
    pub fn identity(&self) -> FontIdentity {
        self.identity
    }
    pub fn metrics(&self) -> &BuiltinMetrics {
        &self.metrics
    }
}
#[derive(Debug, Clone, Copy)]
struct Binding<'doc> {
    value: &'doc Object,
    owner: ObjectId,
    index: usize,
}
#[derive(Debug)]
pub struct PageFontBindings<'doc> {
    page: ObjectId,
    owner: Option<ObjectId>,
    bindings: BTreeMap<&'doc [u8], Binding<'doc>>,
}
impl PageFontBindings<'_> {
    pub fn page(&self) -> ObjectId {
        self.page
    }
    pub fn resource_owner(&self) -> Option<ObjectId> {
        self.owner
    }
    pub fn names(&self) -> impl Iterator<Item = &[u8]> {
        self.bindings.keys().copied()
    }
    pub fn len(&self) -> usize {
        self.bindings.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
    /// Initial reference identity; None inside Some denotes a direct dictionary.
    pub fn target(&self, name: &[u8]) -> Option<Option<ObjectId>> {
        self.bindings.get(name).map(|b| {
            if let Object::Reference(id) = b.value {
                Some(*id)
            } else {
                None
            }
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontResourceLimitKind {
    Calls,
    ReferenceDepth,
    ParentDepth,
    ReferenceHops,
    MetadataItems,
    Bindings,
    Fonts,
    MetricAttempts,
    Metrics,
    BuiltinMetricAttempts,
    BuiltinMetrics,
    BuiltinSlots,
    RetainedBuiltinSlots,
    WidthEntries,
    RetainedWidthEntries,
    Maps,
    NameBytes,
    EncodedBytes,
    DecodedWorkBytes,
    CMapInputBytes,
    CMapEntries,
    CMapPayloadBytes,
    DifferenceItems,
    DifferenceNameBytes,
}
#[derive(Debug)]
pub enum FontResourceErrorKind {
    Document(Box<DocumentError>),
    Pages(Box<PageError>),
    Stream(Box<StreamObjectError>),
    CMap(Box<CMapError>),
    Encoding(Box<EncodingError>),
    Metrics(Box<MetricsError>),
    NotPage,
    MissingFontResource,
    NeedsFontBuiltinEncoding,
    NeedsBuiltinOrFontProgramMetrics,
    ReferenceCycle(ObjectId),
    Invalid(&'static str),
    Unsupported(&'static str),
    LimitExceeded(FontResourceLimitKind),
    AllocationFailed,
}
#[derive(Debug)]
pub struct FontResourceError {
    pub page: Option<ObjectId>,
    pub font: Option<FontIdentity>,
    pub stream: Option<ObjectId>,
    pub field: &'static str,
    pub difference_index: Option<usize>,
    pub width_index: Option<usize>,
    pub kind: FontResourceErrorKind,
}
impl fmt::Display for FontResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "font resources {:?} {:?} stream {:?} {} Differences index {:?} Widths index {:?}: {:?}",
            self.page,
            self.font,
            self.stream,
            self.field,
            self.difference_index,
            self.width_index,
            self.kind
        )
    }
}
impl Error for FontResourceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            FontResourceErrorKind::Document(e) => Some(e),
            FontResourceErrorKind::Pages(e) => Some(e),
            FontResourceErrorKind::Stream(e) => Some(e),
            FontResourceErrorKind::CMap(e) => Some(e),
            FontResourceErrorKind::Encoding(e) => Some(e),
            FontResourceErrorKind::Metrics(e) => Some(e),
            _ => None,
        }
    }
}
fn error(field: &'static str, kind: FontResourceErrorKind) -> FontResourceError {
    FontResourceError {
        page: None,
        font: None,
        stream: None,
        field,
        difference_index: None,
        width_index: None,
        kind,
    }
}
fn invalid(field: &'static str, message: &'static str) -> FontResourceError {
    error(field, FontResourceErrorKind::Invalid(message))
}
fn unsupported(field: &'static str, message: &'static str) -> FontResourceError {
    error(field, FontResourceErrorKind::Unsupported(message))
}
fn charge(
    used: &mut usize,
    amount: usize,
    cap: usize,
    kind: FontResourceLimitKind,
) -> Result<(), FontResourceError> {
    match used.checked_add(amount) {
        Some(n) if n <= cap => {
            *used = n;
            Ok(())
        }
        _ => {
            *used = cap;
            Err(error("budget", FontResourceErrorKind::LimitExceeded(kind)))
        }
    }
}
struct Resolved<'doc> {
    value: &'doc Object,
    id: Option<ObjectId>,
    stream: bool,
}
pub struct FontResourceLoader<'doc, 'input> {
    document: &'doc Document<'input>,
    limits: FontResourceLimits,
    usage: FontResourceUsage,
    pages: BTreeSet<ObjectId>,
    bindings: BTreeMap<ObjectId, PageFontBindings<'doc>>,
    fonts: BTreeMap<FontIdentity, LoadedSimpleFont>,
    maps: BTreeMap<ObjectId, Rc<ToUnicodeMap>>,
    metrics: BTreeMap<FontIdentity, LoadedFontMetrics>,
    builtin_metrics: BTreeMap<FontIdentity, LoadedBuiltinMetrics>,
}
impl<'doc, 'input> FontResourceLoader<'doc, 'input> {
    pub fn new(
        document: &'doc Document<'input>,
        limits: FontResourceLimits,
    ) -> Result<Self, FontResourceError> {
        let pages = collect_pages(document, limits.pages)
            .map_err(|e| error("Pages", FontResourceErrorKind::Pages(Box::new(e))))?
            .into_iter()
            .map(|p| p.id)
            .collect();
        Ok(Self::from_page_ids(document, limits, pages))
    }
    /// Session-only constructor using an inventory already validated against this document.
    pub(crate) fn from_page_ids(
        document: &'doc Document<'input>,
        limits: FontResourceLimits,
        pages: BTreeSet<ObjectId>,
    ) -> Self {
        Self {
            document,
            limits,
            usage: FontResourceUsage::default(),
            pages,
            bindings: BTreeMap::new(),
            fonts: BTreeMap::new(),
            maps: BTreeMap::new(),
            metrics: BTreeMap::new(),
            builtin_metrics: BTreeMap::new(),
        }
    }
    /// Strict page-program prerequisite; ordinary F0 callers are unaffected.
    /// Resolve inherited resources under the same live metadata/reference budget.
    pub(crate) fn validate_page_program_resources(
        &mut self,
        page: ObjectId,
    ) -> Result<(), FontResourceError> {
        self.call()?;
        if !self.pages.contains(&page) {
            return Err(error("Page", FontResourceErrorKind::NotPage));
        }
        let mut current = page;
        for depth in 0..self.limits.max_parent_depth.min(256) {
            charge(
                &mut self.usage.reference_hops,
                1,
                self.limits.max_reference_hops,
                FontResourceLimitKind::ReferenceHops,
            )?;
            let object = self
                .document
                .object(current)
                .map_err(|e| error("Page", FontResourceErrorKind::Document(Box::new(e))))?;
            let Object::Dictionary(dict) = &object.value else {
                return Err(invalid("Page", "expected dictionary"));
            };
            if depth == 0 && self.optional(dict, b"Group", "Page.Group")?.is_some() {
                return Err(unsupported(
                    "Page.Group",
                    "page transparency/group semantics",
                ));
            }
            if let Some(resources) = self.optional(dict, b"Resources", "Resources")? {
                let resources = Self::dictionary(resources, "Resources")?;
                if let Some(colors) =
                    self.optional(resources, b"ColorSpace", "Resources.ColorSpace")?
                {
                    let colors = Self::dictionary(colors, "Resources.ColorSpace")?;
                    for key in [b"DefaultGray".as_slice(), b"DefaultRGB", b"DefaultCMYK"] {
                        if self
                            .optional(colors, key, "Resources.ColorSpace.Default")?
                            .is_some()
                        {
                            return Err(unsupported(
                                "Resources.ColorSpace.Default",
                                "default device color-space override",
                            ));
                        }
                    }
                }
                return Ok(());
            }
            match dict.get(b"Parent".as_slice()) {
                None | Some(Object::Null) => return Ok(()),
                Some(Object::Reference(id)) => current = *id,
                _ => return Err(invalid("Parent", "expected parent reference")),
            }
        }
        Err(error(
            "Parent",
            FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::ParentDepth),
        ))
    }
    pub fn usage(&self) -> FontResourceUsage {
        self.usage
    }
    pub fn loaded(&self, identity: FontIdentity) -> Option<&LoadedSimpleFont> {
        self.fonts.get(&identity)
    }
    fn call(&mut self) -> Result<(), FontResourceError> {
        charge(
            &mut self.usage.calls,
            1,
            self.limits.max_calls,
            FontResourceLimitKind::Calls,
        )
    }
    fn resolve(
        &mut self,
        mut value: &'doc Object,
        field: &'static str,
    ) -> Result<Resolved<'doc>, FontResourceError> {
        let mut seen = [None; 64];
        let mut n = 0;
        let mut final_id = None;
        let mut stream = false;
        while let Object::Reference(id) = value {
            if seen[..n].contains(&Some(*id)) {
                return Err(error(field, FontResourceErrorKind::ReferenceCycle(*id)));
            }
            if n >= self.limits.max_reference_depth.min(64) {
                return Err(error(
                    field,
                    FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::ReferenceDepth),
                ));
            }
            charge(
                &mut self.usage.reference_hops,
                1,
                self.limits.max_reference_hops,
                FontResourceLimitKind::ReferenceHops,
            )?;
            seen[n] = Some(*id);
            n += 1;
            let object = self
                .document
                .object(*id)
                .map_err(|e| error(field, FontResourceErrorKind::Document(Box::new(e))))?;
            value = &object.value;
            stream = object.stream.is_some();
            final_id = Some(*id);
        }
        Ok(Resolved {
            value,
            id: final_id,
            stream,
        })
    }
    fn optional(
        &mut self,
        dict: &'doc Dictionary,
        key: &'static [u8],
        field: &'static str,
    ) -> Result<Option<Resolved<'doc>>, FontResourceError> {
        match dict.get(key) {
            None => Ok(None),
            Some(value) => {
                let r = self.resolve(value, field)?;
                if matches!(r.value, Object::Null) {
                    Ok(None)
                } else {
                    Ok(Some(r))
                }
            }
        }
    }
    fn dictionary(
        resolved: Resolved<'doc>,
        field: &'static str,
    ) -> Result<&'doc Dictionary, FontResourceError> {
        if resolved.stream {
            return Err(invalid(field, "stream where ordinary dictionary required"));
        }
        match resolved.value {
            Object::Dictionary(d) => Ok(d),
            _ => Err(invalid(field, "expected dictionary")),
        }
    }
    fn ensure_bindings(&mut self, page: ObjectId) -> Result<(), FontResourceError> {
        if !self.pages.contains(&page) {
            return Err(error("Page", FontResourceErrorKind::NotPage));
        }
        if self.bindings.contains_key(&page) {
            return Ok(());
        }
        let mut current = page;
        let mut depth = 0usize;
        let mut owner = None;
        let mut font_dict = None;
        loop {
            if depth >= self.limits.max_parent_depth.min(256) {
                return Err(error(
                    "Parent",
                    FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::ParentDepth),
                ));
            }
            depth += 1;
            charge(
                &mut self.usage.reference_hops,
                1,
                self.limits.max_reference_hops,
                FontResourceLimitKind::ReferenceHops,
            )?;
            let object = self
                .document
                .object(current)
                .map_err(|e| error("Parent", FontResourceErrorKind::Document(Box::new(e))))?;
            let Object::Dictionary(d) = &object.value else {
                return Err(invalid("Parent", "expected page-tree dictionary"));
            };
            if let Some(resources) = self.optional(d, b"Resources", "Resources")? {
                let resources = Self::dictionary(resources, "Resources")?;
                owner = Some(current);
                if let Some(fonts) = self.optional(resources, b"Font", "Resources.Font")? {
                    font_dict = Some(Self::dictionary(fonts, "Resources.Font")?);
                }
                break;
            }
            match d.get(b"Parent".as_slice()) {
                None | Some(Object::Null) => break,
                Some(Object::Reference(id)) => current = *id,
                _ => return Err(invalid("Parent", "expected parent reference")),
            }
        }
        let mut bindings = BTreeMap::new();
        if let Some(dict) = font_dict {
            charge(
                &mut self.usage.bindings,
                dict.len(),
                self.limits.max_bindings,
                FontResourceLimitKind::Bindings,
            )?;
            for (index, (name, value)) in dict.iter().enumerate() {
                charge(
                    &mut self.usage.name_bytes,
                    name.len(),
                    self.limits.max_name_bytes,
                    FontResourceLimitKind::NameBytes,
                )?;
                bindings.insert(
                    name.as_slice(),
                    Binding {
                        value,
                        owner: owner.expect("resources owner"),
                        index,
                    },
                );
            }
        }
        self.bindings.insert(
            page,
            PageFontBindings {
                page,
                owner,
                bindings,
            },
        );
        Ok(())
    }
    fn ensure_scope_bindings(
        &mut self,
        scope: &crate::form_resources::ScopedResources<'doc>,
    ) -> Result<ObjectId, FontResourceError> {
        let owner = scope
            .descriptor
            .provider()
            .ok_or_else(|| error("Resources.Font", FontResourceErrorKind::MissingFontResource))?;
        if self.bindings.contains_key(&owner) {
            return Ok(owner);
        }
        let mut bindings = BTreeMap::new();
        if let Some(resources) = scope.resources {
            if let Some(fonts) = self.optional(resources, b"Font", "Resources.Font")? {
                let dict = Self::dictionary(fonts, "Resources.Font")?;
                charge(
                    &mut self.usage.bindings,
                    dict.len(),
                    self.limits.max_bindings,
                    FontResourceLimitKind::Bindings,
                )?;
                for (index, (name, value)) in dict.iter().enumerate() {
                    charge(
                        &mut self.usage.name_bytes,
                        name.len(),
                        self.limits.max_name_bytes,
                        FontResourceLimitKind::NameBytes,
                    )?;
                    bindings.insert(
                        name.as_slice(),
                        Binding {
                            value,
                            owner,
                            index,
                        },
                    );
                }
            }
        }
        self.bindings.insert(
            owner,
            PageFontBindings {
                page: owner,
                owner: Some(owner),
                bindings,
            },
        );
        Ok(owner)
    }
    pub fn page_bindings(
        &mut self,
        page: ObjectId,
    ) -> Result<&PageFontBindings<'doc>, FontResourceError> {
        let run = (|| {
            self.call()?;
            self.ensure_bindings(page)
        })();
        run.map_err(|mut e| {
            e.page = Some(page);
            e
        })?;
        Ok(self.bindings.get(&page).expect("bindings inserted"))
    }
    pub fn load(
        &mut self,
        page: ObjectId,
        name: &[u8],
    ) -> Result<&LoadedSimpleFont, FontResourceError> {
        (|| {
            self.call()?;
            self.ensure_bindings(page)
        })()
        .map_err(|mut e| {
            e.page = Some(page);
            e
        })?;
        self.load_registered(page, name)
    }
    pub(crate) fn load_scoped(
        &mut self,
        scope: &crate::form_resources::ScopedResources<'doc>,
        name: &[u8],
    ) -> Result<&LoadedSimpleFont, FontResourceError> {
        self.call()?;
        let owner = self.ensure_scope_bindings(scope)?;
        self.load_registered(owner, name).map_err(|mut e| {
            e.page = None;
            e
        })
    }
    fn load_registered(
        &mut self,
        page: ObjectId,
        name: &[u8],
    ) -> Result<&LoadedSimpleFont, FontResourceError> {
        let result: Result<FontIdentity, FontResourceError> = (|| {
            let binding = *self.bindings[&page].bindings.get(name).ok_or_else(|| {
                error("Resources.Font", FontResourceErrorKind::MissingFontResource)
            })?;
            let resolved = self.resolve(binding.value, "Font")?;
            let identity =
                resolved
                    .id
                    .map(FontIdentity::Indirect)
                    .unwrap_or(FontIdentity::Direct {
                        resource_owner: binding.owner,
                        resource_index: binding.index,
                    });
            if self.fonts.contains_key(&identity) {
                return Ok(identity);
            }
            let dict = Self::dictionary(resolved, "Font")?;
            let loaded = self.load_font(dict, identity).map_err(|mut e| {
                e.font = Some(identity);
                e
            })?;
            self.fonts.insert(identity, loaded);
            Ok(identity)
        })();
        let identity = result.map_err(|mut e| {
            e.page = Some(page);
            e
        })?;
        Ok(self.fonts.get(&identity).expect("font inserted"))
    }
    pub fn loaded_metrics(&self, identity: FontIdentity) -> Option<&LoadedFontMetrics> {
        self.metrics.get(&identity)
    }
    /// Load advances independently of the Unicode mapping cache. Unsupported or
    /// malformed ToUnicode/Encoding does not alter explicit width declarations.
    pub fn load_metrics(
        &mut self,
        page: ObjectId,
        name: &[u8],
    ) -> Result<&LoadedFontMetrics, FontResourceError> {
        (|| {
            self.call()?;
            charge(
                &mut self.usage.metric_attempts,
                1,
                self.limits.max_metric_attempts,
                FontResourceLimitKind::MetricAttempts,
            )?;
            self.ensure_bindings(page)
        })()
        .map_err(|mut e| {
            e.page = Some(page);
            e
        })?;
        self.load_metrics_registered(page, name)
    }
    pub(crate) fn load_metrics_scoped(
        &mut self,
        scope: &crate::form_resources::ScopedResources<'doc>,
        name: &[u8],
    ) -> Result<&LoadedFontMetrics, FontResourceError> {
        self.call()?;
        charge(
            &mut self.usage.metric_attempts,
            1,
            self.limits.max_metric_attempts,
            FontResourceLimitKind::MetricAttempts,
        )?;
        let owner = self.ensure_scope_bindings(scope)?;
        self.load_metrics_registered(owner, name).map_err(|mut e| {
            e.page = None;
            e
        })
    }
    fn load_metrics_registered(
        &mut self,
        page: ObjectId,
        name: &[u8],
    ) -> Result<&LoadedFontMetrics, FontResourceError> {
        let result: Result<FontIdentity, FontResourceError> = (|| {
            let binding = *self.bindings[&page].bindings.get(name).ok_or_else(|| {
                error("Resources.Font", FontResourceErrorKind::MissingFontResource)
            })?;
            let resolved = self.resolve(binding.value, "Font")?;
            let identity =
                resolved
                    .id
                    .map(FontIdentity::Indirect)
                    .unwrap_or(FontIdentity::Direct {
                        resource_owner: binding.owner,
                        resource_index: binding.index,
                    });
            if self.metrics.contains_key(&identity) {
                return Ok(identity);
            }
            let dict = Self::dictionary(resolved, "Font")?;
            let metrics = self.explicit_metrics(dict).map_err(|mut e| {
                e.font = Some(identity);
                e
            })?;
            self.metrics
                .insert(identity, LoadedFontMetrics { identity, metrics });
            Ok(identity)
        })();
        let identity = result.map_err(|mut e| {
            e.page = Some(page);
            e
        })?;
        Ok(self.metrics.get(&identity).expect("metrics inserted"))
    }
    pub fn loaded_builtin_metrics(&self, identity: FontIdentity) -> Option<&LoadedBuiltinMetrics> {
        self.builtin_metrics.get(&identity)
    }
    /// Resolve only exact supported builtin declarations. Explicit Widths never
    /// fall through this path, and ToUnicode is not used for glyph selection.
    pub fn load_builtin_metrics(
        &mut self,
        page: ObjectId,
        name: &[u8],
    ) -> Result<&LoadedBuiltinMetrics, FontResourceError> {
        (|| {
            self.call()?;
            charge(
                &mut self.usage.builtin_metric_attempts,
                1,
                self.limits.max_builtin_metric_attempts,
                FontResourceLimitKind::BuiltinMetricAttempts,
            )?;
            self.ensure_bindings(page)
        })()
        .map_err(|mut e| {
            e.page = Some(page);
            e
        })?;
        self.load_builtin_metrics_registered(page, name)
    }
    pub(crate) fn load_builtin_metrics_scoped(
        &mut self,
        scope: &crate::form_resources::ScopedResources<'doc>,
        name: &[u8],
    ) -> Result<&LoadedBuiltinMetrics, FontResourceError> {
        self.call()?;
        charge(
            &mut self.usage.builtin_metric_attempts,
            1,
            self.limits.max_builtin_metric_attempts,
            FontResourceLimitKind::BuiltinMetricAttempts,
        )?;
        let owner = self.ensure_scope_bindings(scope)?;
        self.load_builtin_metrics_registered(owner, name)
            .map_err(|mut e| {
                e.page = None;
                e
            })
    }
    fn load_builtin_metrics_registered(
        &mut self,
        page: ObjectId,
        name: &[u8],
    ) -> Result<&LoadedBuiltinMetrics, FontResourceError> {
        let result: Result<FontIdentity, FontResourceError> = (|| {
            let binding = *self.bindings[&page].bindings.get(name).ok_or_else(|| {
                error("Resources.Font", FontResourceErrorKind::MissingFontResource)
            })?;
            let resolved = self.resolve(binding.value, "Font")?;
            let identity =
                resolved
                    .id
                    .map(FontIdentity::Indirect)
                    .unwrap_or(FontIdentity::Direct {
                        resource_owner: binding.owner,
                        resource_index: binding.index,
                    });
            if self.builtin_metrics.contains_key(&identity) {
                return Ok(identity);
            }
            let dict = Self::dictionary(resolved, "Font")?;
            let metrics = self.build_builtin_metrics(dict).map_err(|mut e| {
                e.font = Some(identity);
                e
            })?;
            self.builtin_metrics
                .insert(identity, LoadedBuiltinMetrics { identity, metrics });
            Ok(identity)
        })();
        let identity = result.map_err(|mut e| {
            e.page = Some(page);
            e
        })?;
        Ok(self
            .builtin_metrics
            .get(&identity)
            .expect("builtin metrics inserted"))
    }
    fn build_builtin_metrics(
        &mut self,
        dict: &'doc Dictionary,
    ) -> Result<BuiltinMetrics, FontResourceError> {
        if self.usage.builtin_metrics >= self.limits.max_builtin_metrics {
            return Err(error(
                "BuiltinMetrics",
                FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::BuiltinMetrics),
            ));
        }
        self.check_type(dict, b"Font", "Font.Type")?;
        match self.metric_optional(dict, b"Subtype", "Font.Subtype")? {
            Some(Resolved {
                value: Object::Name(n),
                stream: false,
                ..
            }) if n == b"Type1" => {}
            Some(Resolved {
                value: Object::Name(_),
                stream: false,
                ..
            }) => {
                return Err(unsupported(
                    "Font.Subtype",
                    "builtin metrics require exact Type1 subtype",
                ));
            }
            _ => return Err(invalid("Font.Subtype", "missing or invalid subtype")),
        }
        let face = match self.metric_optional(dict, b"BaseFont", "Font.BaseFont")? {
            Some(Resolved {
                value: Object::Name(n),
                stream: false,
                ..
            }) => BuiltinFace::from_pdf_name(n).ok_or_else(|| {
                unsupported(
                    "Font.BaseFont",
                    "only exact Helvetica and Times-Roman builtin metrics",
                )
            })?,
            _ => return Err(invalid("Font.BaseFont", "expected exact builtin name")),
        };
        if self
            .metric_optional(dict, b"Widths", "Font.Widths")?
            .is_some()
        {
            return Err(unsupported(
                "Font.Widths",
                "non-null explicit Widths excludes builtin metrics",
            ));
        }
        if self
            .metric_optional(dict, b"FirstChar", "Font.FirstChar")?
            .is_some()
            || self
                .metric_optional(dict, b"LastChar", "Font.LastChar")?
                .is_some()
        {
            return Err(invalid("Font.Widths", "endpoints without Widths"));
        }
        if let Some(descriptor) =
            self.metric_optional(dict, b"FontDescriptor", "Font.FontDescriptor")?
        {
            let descriptor = Self::dictionary(descriptor, "Font.FontDescriptor")?;
            self.check_type(descriptor, b"FontDescriptor", "Font.FontDescriptor.Type")?;
            for key in [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"] {
                if self
                    .metric_optional(descriptor, key, "Font.FontDescriptor.EmbeddedProgram")?
                    .is_some()
                {
                    return Err(unsupported(
                        "Font.FontDescriptor.EmbeddedProgram",
                        "embedded program excludes builtin metrics",
                    ));
                }
            }
        }
        charge(
            &mut self.usage.builtin_slots,
            256,
            self.limits.max_builtin_slots,
            FontResourceLimitKind::BuiltinSlots,
        )?;
        if 256 > self.limits.max_retained_builtin_slots - self.usage.retained_builtin_slots {
            return Err(error(
                "BuiltinMetrics",
                FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::RetainedBuiltinSlots),
            ));
        }
        // This uses explicit Encoding even if a ToUnicode entry is present or
        // malformed; Unicode interpretation is a separate lazy capability.
        let mapping = self.encoding(dict)?;
        let SimpleFontMapping::ExplicitEncoding { encoding, .. } = mapping else {
            unreachable!("explicit encoding helper")
        };
        let result = BuiltinMetrics::from_boxed_encoding(face, encoding);
        self.usage.builtin_metrics += 1;
        self.usage.retained_builtin_slots += 256;
        Ok(result)
    }
    fn metric_optional(
        &mut self,
        dict: &'doc Dictionary,
        key: &'static [u8],
        field: &'static str,
    ) -> Result<Option<Resolved<'doc>>, FontResourceError> {
        charge(
            &mut self.usage.metadata_items,
            1,
            self.limits.max_metadata_items,
            FontResourceLimitKind::MetadataItems,
        )?;
        self.optional(dict, key, field)
    }
    fn metric_number(
        &mut self,
        value: &'doc Object,
        field: &'static str,
    ) -> Result<FontAdvance, FontResourceError> {
        charge(
            &mut self.usage.metadata_items,
            1,
            self.limits.max_metadata_items,
            FontResourceLimitKind::MetadataItems,
        )?;
        let resolved = self.resolve(value, field)?;
        if resolved.stream {
            return Err(invalid(field, "stream is not a numeric width"));
        }
        let number = match resolved.value {
            Object::Integer(n) => FontAdvance::from_integer(*n),
            Object::Real(n) => FontAdvance::new(*n),
            _ => return Err(invalid(field, "expected numeric width")),
        };
        number.map_err(|e| error(field, FontResourceErrorKind::Metrics(Box::new(e))))
    }
    fn explicit_metrics(
        &mut self,
        dict: &'doc Dictionary,
    ) -> Result<HorizontalMetrics, FontResourceError> {
        if self.usage.metrics >= self.limits.max_metrics {
            return Err(error(
                "Widths",
                FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::Metrics),
            ));
        }
        self.check_type(dict, b"Font", "Font.Type")?;
        match self.metric_optional(dict, b"Subtype", "Font.Subtype")? {
            Some(Resolved {
                value: Object::Name(n),
                stream: false,
                ..
            }) if n == b"Type1" || n == b"TrueType" => {}
            Some(Resolved {
                value: Object::Name(n),
                stream: false,
                ..
            }) if n == b"Type0" => return self.cid_metrics(dict),
            Some(Resolved {
                value: Object::Name(_),
                stream: false,
                ..
            }) => {
                return Err(unsupported(
                    "Font.Subtype",
                    "only Type1, TrueType and Type0 metrics are supported",
                ));
            }
            _ => return Err(invalid("Font.Subtype", "missing or invalid subtype")),
        }
        let widths = self.metric_optional(dict, b"Widths", "Font.Widths")?;
        let first = self.metric_optional(dict, b"FirstChar", "Font.FirstChar")?;
        let last = self.metric_optional(dict, b"LastChar", "Font.LastChar")?;
        let Some(widths) = widths else {
            if first.is_some() || last.is_some() {
                return Err(invalid("Font.Widths", "endpoints without Widths"));
            }
            return Err(error(
                "Font.Widths",
                FontResourceErrorKind::NeedsBuiltinOrFontProgramMetrics,
            ));
        };
        let endpoint = |value: Option<Resolved<'doc>>, field| match value {
            Some(Resolved {
                value: Object::Integer(n),
                stream: false,
                ..
            }) => u8::try_from(*n).map_err(|_| invalid(field, "endpoint must be in 0..255")),
            _ => Err(invalid(field, "expected integer endpoint")),
        };
        let first = endpoint(first, "Font.FirstChar")?;
        let last = endpoint(last, "Font.LastChar")?;
        if first > last {
            return Err(invalid("Font.Widths", "FirstChar exceeds LastChar"));
        }
        if widths.stream {
            return Err(invalid("Font.Widths", "stream is not Widths array"));
        }
        let Object::Array(items) = widths.value else {
            return Err(invalid("Font.Widths", "expected Widths array"));
        };
        // Charge each requested entry before inspecting the array contents,
        // including malformed/count-mismatched arrays and failed retries.
        charge(
            &mut self.usage.width_entries,
            items.len(),
            self.limits.max_width_entries,
            FontResourceLimitKind::WidthEntries,
        )?;
        let expected = usize::from(last) - usize::from(first) + 1;
        if items.len() != expected {
            return Err(error(
                "Font.Widths",
                FontResourceErrorKind::Metrics(Box::new(MetricsError {
                    index: None,
                    kind: crate::font_metrics::MetricsErrorKind::WrongWidthCount {
                        expected,
                        actual: items.len(),
                    },
                })),
            ));
        }
        if expected > self.limits.max_retained_width_entries - self.usage.retained_width_entries {
            return Err(error(
                "Font.Widths",
                FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::RetainedWidthEntries),
            ));
        }
        let missing = match self.metric_optional(dict, b"FontDescriptor", "Font.FontDescriptor")? {
            None => None,
            Some(descriptor) => {
                let descriptor = Self::dictionary(descriptor, "Font.FontDescriptor")?;
                match self.metric_optional(
                    descriptor,
                    b"MissingWidth",
                    "Font.FontDescriptor.MissingWidth",
                )? {
                    None => None,
                    Some(r) => {
                        if r.stream {
                            return Err(invalid(
                                "Font.FontDescriptor.MissingWidth",
                                "stream is not number",
                            ));
                        }
                        Some(self.metric_number(r.value, "Font.FontDescriptor.MissingWidth")?)
                    }
                }
            }
        };
        // Fixed temporary storage avoids a second dynamically growing array.
        let mut values = [FontAdvance::new(0.0).expect("finite zero"); 256];
        for (index, item) in items.iter().enumerate() {
            values[index] = self.metric_number(item, "Font.Widths").map_err(|mut e| {
                e.width_index = Some(index);
                if let FontResourceErrorKind::Metrics(ref mut inner) = e.kind {
                    inner.index = Some(index);
                }
                e
            })?;
        }
        let result = HorizontalMetrics::from_widths(first, last, &values[..expected], missing)
            .map_err(|e| error("Font.Widths", FontResourceErrorKind::Metrics(Box::new(e))))?;
        self.usage.metrics += 1;
        self.usage.retained_width_entries += expected;
        Ok(result)
    }
    /// The single CIDFontType2 descendant of an Identity-H Type0 font. Other
    /// CMaps, vertical writing and CFF-based CIDFonts are not supported yet.
    fn descendant(
        &mut self,
        font: &'doc Dictionary,
    ) -> Result<&'doc Dictionary, FontResourceError> {
        match self.optional(font, b"Encoding", "Font.Encoding")? {
            Some(Resolved {
                value: Object::Name(name),
                stream: false,
                ..
            }) if name == b"Identity-H" => {}
            Some(Resolved {
                value: Object::Name(name),
                stream: false,
                ..
            }) if name == b"Identity-V" => {
                return Err(unsupported(
                    "Font.Encoding",
                    "vertical writing (Identity-V)",
                ));
            }
            Some(_) => {
                return Err(unsupported(
                    "Font.Encoding",
                    "only the Identity-H CMap is supported",
                ));
            }
            None => return Err(invalid("Font.Encoding", "Type0 font requires Encoding")),
        }
        let fonts = self
            .optional(font, b"DescendantFonts", "Font.DescendantFonts")?
            .ok_or_else(|| {
                invalid(
                    "Font.DescendantFonts",
                    "Type0 font requires DescendantFonts",
                )
            })?;
        let item = match fonts {
            Resolved {
                value: Object::Array(items),
                stream: false,
                ..
            } if items.len() == 1 => &items[0],
            _ => {
                return Err(invalid(
                    "Font.DescendantFonts",
                    "expected a one-element array",
                ));
            }
        };
        let resolved = self.resolve(item, "Font.DescendantFonts")?;
        let descendant = Self::dictionary(resolved, "Font.DescendantFonts")?;
        self.check_type(descendant, b"Font", "Font.DescendantFonts.Type")?;
        match self.optional(descendant, b"Subtype", "Font.DescendantFonts.Subtype")? {
            Some(Resolved {
                value: Object::Name(n),
                stream: false,
                ..
            }) if n == b"CIDFontType2" => Ok(descendant),
            Some(Resolved {
                value: Object::Name(n),
                stream: false,
                ..
            }) if n == b"CIDFontType0" => Err(unsupported(
                "Font.DescendantFonts.Subtype",
                "CFF-based CIDFontType0 fonts",
            )),
            _ => Err(invalid(
                "Font.DescendantFonts.Subtype",
                "expected CIDFontType2 or CIDFontType0",
            )),
        }
    }
    /// CIDFont /W and /DW. /W is [c [w...]] or [cfirst clast w] groups.
    fn cid_metrics(
        &mut self,
        font: &'doc Dictionary,
    ) -> Result<HorizontalMetrics, FontResourceError> {
        let descendant = self.descendant(font)?;
        let default = match self.metric_optional(descendant, b"DW", "Font.DescendantFonts.DW")? {
            None => None,
            Some(r) if !r.stream => Some(self.metric_number(r.value, "Font.DescendantFonts.DW")?),
            Some(_) => return Err(invalid("Font.DescendantFonts.DW", "stream is not number")),
        };
        let mut ranges = Vec::new();
        if let Some(w) = self.metric_optional(descendant, b"W", "Font.DescendantFonts.W")? {
            let items = match w {
                Resolved {
                    value: Object::Array(items),
                    stream: false,
                    ..
                } => items,
                _ => return Err(invalid("Font.DescendantFonts.W", "expected array")),
            };
            charge(
                &mut self.usage.width_entries,
                items.len(),
                self.limits.max_width_entries,
                FontResourceLimitKind::WidthEntries,
            )?;
            let cid = |r: Resolved<'doc>| match r {
                Resolved {
                    value: Object::Integer(n),
                    stream: false,
                    ..
                } => u16::try_from(*n)
                    .map(u32::from)
                    .map_err(|_| invalid("Font.DescendantFonts.W", "CID outside 0..65535")),
                _ => Err(invalid("Font.DescendantFonts.W", "expected integer CID")),
            };
            let mut i = 0;
            while i < items.len() {
                let first = cid(self.resolve(&items[i], "Font.DescendantFonts.W")?)?;
                let next = items
                    .get(i + 1)
                    .ok_or_else(|| invalid("Font.DescendantFonts.W", "truncated W group"))?;
                let next = self.resolve(next, "Font.DescendantFonts.W")?;
                if let Resolved {
                    value: Object::Array(widths),
                    stream: false,
                    ..
                } = next
                {
                    for (offset, width) in widths.iter().enumerate() {
                        let cid = first + offset as u32;
                        if cid > 0xffff {
                            return Err(invalid("Font.DescendantFonts.W", "CID outside 0..65535"));
                        }
                        let advance = self.metric_number(width, "Font.DescendantFonts.W")?;
                        self.cid_range(&mut ranges, cid, cid, advance)?;
                    }
                    i += 2;
                } else {
                    let last = cid(next)?;
                    let width = items
                        .get(i + 2)
                        .ok_or_else(|| invalid("Font.DescendantFonts.W", "truncated W group"))?;
                    let advance = self.metric_number(width, "Font.DescendantFonts.W")?;
                    if first > last {
                        return Err(invalid("Font.DescendantFonts.W", "reversed CID range"));
                    }
                    self.cid_range(&mut ranges, first, last, advance)?;
                    i += 3;
                }
            }
        }
        let count = ranges.len();
        let result = HorizontalMetrics::from_cid_widths(ranges, default).map_err(|e| {
            error(
                "Font.DescendantFonts.W",
                FontResourceErrorKind::Metrics(Box::new(e)),
            )
        })?;
        self.usage.metrics += 1;
        self.usage.retained_width_entries += count;
        Ok(result)
    }
    fn cid_range(
        &mut self,
        ranges: &mut Vec<CidWidthRange>,
        first: u32,
        last: u32,
        advance: FontAdvance,
    ) -> Result<(), FontResourceError> {
        // Adjacent equal widths merge, so [c [w w w]] costs one retained range.
        if let Some(prior) = ranges.last_mut() {
            if prior.last + 1 == first && prior.advance == advance {
                prior.last = last;
                return Ok(());
            }
        }
        if ranges.len()
            >= self.limits.max_retained_width_entries - self.usage.retained_width_entries
        {
            return Err(error(
                "Font.DescendantFonts.W",
                FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::RetainedWidthEntries),
            ));
        }
        ranges
            .try_reserve(1)
            .map_err(|_| invalid("Font.DescendantFonts.W", "allocation failed"))?;
        ranges.push(CidWidthRange {
            first,
            last,
            advance,
        });
        Ok(())
    }
    fn check_type(
        &mut self,
        dict: &'doc Dictionary,
        wanted: &[u8],
        field: &'static str,
    ) -> Result<(), FontResourceError> {
        if let Some(r) = self.optional(dict, b"Type", field)? {
            if r.stream || !matches!(r.value,Object::Name(n) if n==wanted) {
                return Err(invalid(field, "unexpected Type"));
            }
        }
        Ok(())
    }
    fn load_font(
        &mut self,
        dict: &'doc Dictionary,
        identity: FontIdentity,
    ) -> Result<LoadedSimpleFont, FontResourceError> {
        if self.usage.fonts >= self.limits.max_fonts {
            return Err(error(
                "Font",
                FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::Fonts),
            ));
        }
        self.check_type(dict, b"Font", "Font.Type")?;
        let subtype = match self.optional(dict, b"Subtype", "Font.Subtype")? {
            Some(Resolved {
                value: Object::Name(name),
                stream: false,
                ..
            }) => match name.as_slice() {
                b"Type1" => SimpleFontSubtype::Type1,
                b"TrueType" => SimpleFontSubtype::TrueType,
                b"Type0" => SimpleFontSubtype::Type0,
                _ => {
                    return Err(unsupported(
                        "Font.Subtype",
                        "only Type1, TrueType and Type0 fonts are supported",
                    ));
                }
            },
            _ => return Err(invalid("Font.Subtype", "missing or invalid subtype")),
        };
        let base_font = match self.optional(dict, b"BaseFont", "Font.BaseFont")? {
            None => None,
            Some(Resolved {
                value: Object::Name(name),
                stream: false,
                ..
            }) => {
                charge(
                    &mut self.usage.name_bytes,
                    name.len(),
                    self.limits.max_name_bytes,
                    FontResourceLimitKind::NameBytes,
                )?;
                Some(name.clone())
            }
            _ => return Err(invalid("Font.BaseFont", "expected name")),
        };
        if subtype == SimpleFontSubtype::Type0 {
            self.descendant(dict)?;
        }
        let mapping = if let Some(r) = self.optional(dict, b"ToUnicode", "Font.ToUnicode")? {
            if matches!(r.value, Object::Name(_)) {
                return Err(unsupported("Font.ToUnicode", "predefined ToUnicode map"));
            }
            if !r.stream {
                return Err(invalid("Font.ToUnicode", "expected indirect stream"));
            }
            let id =
                r.id.ok_or_else(|| invalid("Font.ToUnicode", "stream needs identity"))?;
            let width = if subtype == SimpleFontSubtype::Type0 {
                CodeWidth::Two
            } else {
                CodeWidth::One
            };
            let map = self.load_map(id, width).map_err(|mut e| {
                e.stream = Some(id);
                e
            })?;
            SimpleFontMapping::ToUnicode(map)
        } else if subtype == SimpleFontSubtype::Type0 {
            // Identity CIDs carry no Unicode; extraction and editing need ToUnicode.
            return Err(unsupported(
                "Font.ToUnicode",
                "Type0 font without ToUnicode",
            ));
        } else {
            self.encoding(dict)?
        };
        self.usage.fonts += 1;
        Ok(LoadedSimpleFont {
            identity,
            subtype,
            base_font,
            mapping,
        })
    }
    // Inspect exactly the metadata shapes the stream adapter may traverse, so
    // indirect parameter work cannot escape this session's counters. Charge a
    // second traversal before decoding; decoder failures expose no hop usage.
    fn stream_metadata(&mut self, dict: &'doc Dictionary) -> Result<(), FontResourceError> {
        let before = self.usage.reference_hops;
        for key in [b"F".as_slice(), b"Filter", b"DecodeParms"] {
            if let Some(value) = dict.get(key) {
                let resolved = self.metadata_value(value)?;
                if key == b"Filter" {
                    if let Object::Array(items) = resolved {
                        for item in items {
                            self.metadata_value(item)?;
                        }
                    }
                } else if key == b"DecodeParms" {
                    match resolved {
                        Object::Dictionary(parameters) => self.parameter_metadata(parameters)?,
                        Object::Array(items) => {
                            for item in items {
                                if let Object::Dictionary(parameters) = self.metadata_value(item)? {
                                    self.parameter_metadata(parameters)?;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        let hops = self.usage.reference_hops - before;
        charge(
            &mut self.usage.reference_hops,
            hops,
            self.limits.max_reference_hops,
            FontResourceLimitKind::ReferenceHops,
        )
    }
    fn metadata_value(&mut self, value: &'doc Object) -> Result<&'doc Object, FontResourceError> {
        charge(
            &mut self.usage.metadata_items,
            1,
            self.limits.max_metadata_items,
            FontResourceLimitKind::MetadataItems,
        )?;
        let resolved = self.resolve(value, "ToUnicode.metadata")?;
        if resolved.stream {
            return Err(invalid(
                "ToUnicode.metadata",
                "metadata reference targets stream",
            ));
        }
        Ok(resolved.value)
    }
    fn parameter_metadata(&mut self, dict: &'doc Dictionary) -> Result<(), FontResourceError> {
        for value in dict.values() {
            self.metadata_value(value)?;
        }
        Ok(())
    }
    /// `width` is the font's code width: one byte for simple fonts, two for
    /// Identity-H Type0. A cached map is rechecked against each user's width.
    fn load_map(
        &mut self,
        id: ObjectId,
        width: CodeWidth,
    ) -> Result<Rc<ToUnicodeMap>, FontResourceError> {
        let check = |map: &ToUnicodeMap| {
            if map.writing_mode() != 0 || map.codespaces().iter().any(|s| s.low().width() != width)
            {
                return Err(unsupported(
                    "ToUnicode",
                    "map code width differs from the font's horizontal code width",
                ));
            }
            Ok(())
        };
        if let Some(map) = self.maps.get(&id) {
            check(map)?;
            return Ok(Rc::clone(map));
        }
        if self.usage.maps >= self.limits.max_maps {
            return Err(error(
                "ToUnicode",
                FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::Maps),
            ));
        }
        let object = self
            .document
            .object(id)
            .map_err(|e| error("ToUnicode", FontResourceErrorKind::Document(Box::new(e))))?;
        let Object::Dictionary(dict) = &object.value else {
            return Err(invalid("ToUnicode", "expected stream dictionary"));
        };
        if self
            .optional(dict, b"UseCMap", "ToUnicode.UseCMap")?
            .is_some()
        {
            return Err(unsupported("ToUnicode.UseCMap", "base CMap dependency"));
        }
        self.stream_metadata(dict)?;
        let bytes = self
            .document
            .stream_data(id)
            .map_err(|e| error("ToUnicode", FontResourceErrorKind::Document(Box::new(e))))?
            .ok_or_else(|| invalid("ToUnicode", "expected stream"))?;
        charge(
            &mut self.usage.encoded_bytes,
            bytes.len(),
            self.limits.max_encoded_bytes,
            FontResourceLimitKind::EncodedBytes,
        )?;
        let work = self
            .limits
            .stream
            .max_total_decoded_bytes
            .min(self.limits.max_decoded_work_bytes - self.usage.decoded_work_bytes);
        let input_cap = self
            .limits
            .cmap
            .max_input_bytes
            .min(self.limits.max_cmap_input_bytes - self.usage.cmap_input_bytes);
        let stream_limits = StreamLimits {
            max_encoded_bytes: self.limits.stream.max_encoded_bytes,
            max_decoded_bytes: self.limits.stream.max_decoded_bytes.min(input_cap),
            max_total_decoded_bytes: work,
            ..self.limits.stream
        };
        charge(
            &mut self.usage.decoded_work_bytes,
            work,
            self.limits.max_decoded_work_bytes,
            FontResourceLimitKind::DecodedWorkBytes,
        )?;
        let decoded = decode_document_stream_with_usage(self.document, id, stream_limits)
            .map_err(|e| error("ToUnicode", FontResourceErrorKind::Stream(Box::new(e))))?;
        self.usage.decoded_work_bytes -= work - decoded.usage.total_decoded_bytes;
        charge(
            &mut self.usage.cmap_input_bytes,
            decoded.bytes.len(),
            self.limits.max_cmap_input_bytes,
            FontResourceLimitKind::CMapInputBytes,
        )?;
        let entries = self
            .limits
            .cmap
            .max_mappings
            .min(self.limits.max_cmap_entries - self.usage.cmap_entries);
        let payload = self
            .limits
            .cmap
            .max_owned_bytes
            .min(self.limits.max_cmap_payload_bytes - self.usage.cmap_payload_bytes);
        charge(
            &mut self.usage.cmap_entries,
            entries,
            self.limits.max_cmap_entries,
            FontResourceLimitKind::CMapEntries,
        )?;
        charge(
            &mut self.usage.cmap_payload_bytes,
            payload,
            self.limits.max_cmap_payload_bytes,
            FontResourceLimitKind::CMapPayloadBytes,
        )?;
        let map = parse_to_unicode(
            &decoded.bytes,
            CMapLimits {
                max_input_bytes: input_cap,
                max_mappings: entries,
                max_owned_bytes: payload,
                ..self.limits.cmap
            },
        )
        .map_err(|e| error("ToUnicode", FontResourceErrorKind::CMap(Box::new(e))))?;
        // Include source bytes conservatively for discarded metadata/name work. Every
        // scalar is at most four owned bytes and the parser's own cap bounds the sum.
        let actual = map
            .mappings()
            .try_fold(map.name().map_or(0, <[u8]>::len), |n, (_, s)| {
                n.checked_add(s.scalars().len().checked_mul(4)?)
            })
            .ok_or_else(|| {
                error(
                    "ToUnicode",
                    FontResourceErrorKind::LimitExceeded(FontResourceLimitKind::CMapPayloadBytes),
                )
            })?;
        check(&map)?;
        self.usage.cmap_entries -= entries - map.len();
        self.usage.cmap_payload_bytes -=
            payload - actual.saturating_add(decoded.bytes.len()).min(payload);
        let map = Rc::new(map);
        self.maps.insert(id, Rc::clone(&map));
        self.usage.maps += 1;
        Ok(map)
    }
    fn encoding(&mut self, font: &'doc Dictionary) -> Result<SimpleFontMapping, FontResourceError> {
        let r = self
            .optional(font, b"Encoding", "Font.Encoding")?
            .ok_or_else(|| {
                error(
                    "Font.Encoding",
                    FontResourceErrorKind::NeedsFontBuiltinEncoding,
                )
            })?;
        let (base, differences) = match r {
            Resolved {
                value: Object::Name(name),
                stream: false,
                ..
            } => (
                NamedEncoding::from_pdf_name(name)
                    .ok_or_else(|| unsupported("Font.Encoding", "unknown named encoding"))?,
                None,
            ),
            other => {
                let d = Self::dictionary(other, "Font.Encoding")?;
                self.check_type(d, b"Encoding", "Font.Encoding.Type")?;
                let base = self
                    .optional(d, b"BaseEncoding", "Font.Encoding.BaseEncoding")?
                    .ok_or_else(|| {
                        error(
                            "Font.Encoding.BaseEncoding",
                            FontResourceErrorKind::NeedsFontBuiltinEncoding,
                        )
                    })?;
                let base = match base {
                    Resolved {
                        value: Object::Name(n),
                        stream: false,
                        ..
                    } => NamedEncoding::from_pdf_name(n).ok_or_else(|| {
                        unsupported("Font.Encoding.BaseEncoding", "unknown base encoding")
                    })?,
                    _ => return Err(invalid("Font.Encoding.BaseEncoding", "expected name")),
                };
                (
                    base,
                    self.optional(d, b"Differences", "Font.Encoding.Differences")?,
                )
            }
        };
        let mut encoding = SimpleEncoding::from_named(base);
        if let Some(r) = differences {
            if r.stream {
                return Err(invalid("Font.Encoding.Differences", "stream is not array"));
            }
            let Object::Array(items) = r.value else {
                return Err(invalid("Font.Encoding.Differences", "expected array"));
            };
            charge(
                &mut self.usage.difference_items,
                items.len(),
                self.limits.max_difference_items,
                FontResourceLimitKind::DifferenceItems,
            )?;
            if items.len() > self.limits.encoding.max_items {
                return Err(error(
                    "Font.Encoding.Differences",
                    FontResourceErrorKind::Encoding(Box::new(EncodingError {
                        index: None,
                        kind: EncodingErrorKind::LimitExceeded("Differences items"),
                    })),
                ));
            }
            let mut entries = Vec::new();
            entries.try_reserve_exact(items.len()).map_err(|_| {
                error(
                    "Font.Encoding.Differences",
                    FontResourceErrorKind::AllocationFailed,
                )
            })?;
            for (i, item) in items.iter().enumerate() {
                let resolved =
                    self.resolve(item, "Font.Encoding.Differences")
                        .map_err(|mut e| {
                            e.difference_index = Some(i);
                            e
                        })?;
                let bad = |kind| {
                    error(
                        "Font.Encoding.Differences",
                        FontResourceErrorKind::Encoding(Box::new(EncodingError {
                            index: Some(i),
                            kind,
                        })),
                    )
                };
                if resolved.stream {
                    return Err(bad(EncodingErrorKind::ExpectedCodeOrName));
                }
                let difference = match resolved.value {
                    Object::Integer(n) => Difference::Code(
                        u8::try_from(*n).map_err(|_| bad(EncodingErrorKind::CodeOutOfRange))?,
                    ),
                    Object::Name(n) => {
                        charge(
                            &mut self.usage.difference_name_bytes,
                            n.len(),
                            self.limits.max_difference_name_bytes,
                            FontResourceLimitKind::DifferenceNameBytes,
                        )
                        .map_err(|mut e| {
                            e.difference_index = Some(i);
                            e
                        })?;
                        Difference::Name(n)
                    }
                    _ => return Err(bad(EncodingErrorKind::ExpectedCodeOrName)),
                };
                entries.push(difference);
            }
            encoding = encoding
                .with_differences(&entries, self.limits.encoding)
                .map_err(|e| {
                    error(
                        "Font.Encoding.Differences",
                        FontResourceErrorKind::Encoding(Box::new(e)),
                    )
                })?;
        }
        Ok(SimpleFontMapping::ExplicitEncoding {
            base,
            encoding: Box::new(encoding),
        })
    }
}
