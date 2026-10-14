// SPDX-License-Identifier: AGPL-3.0-or-later
//! Document-bound Form metadata, scoped resources and owned execution provenance.
//! Decoded buffers use PP1's content partition; metadata, handles and diagnostics
//! use the separate cumulative Form partition. Only active ancestry is cyclic.
//! name_bytes counts admitted resource-name bytes visited; allocated_bytes is a
//! distinct conservative full-request ledger. Both successful and failed work is
//! cumulative; retained unique metadata admissions are not invocation counts.
use crate::{
    document::{Document, DocumentError},
    geometry::{Affine, GeometryError, Rect, UserSpace},
    object::{Object, ObjectId},
};
use std::{collections::BTreeMap, fmt, mem::size_of, ops::Range, sync::Arc};
type Dictionary = BTreeMap<Vec<u8>, Object>;
macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
        pub struct $name(pub(crate) usize);
        impl $name {
            pub fn index(self) -> usize {
                self.0
            }
        }
    };
}
id!(ResourceScopeId);
id!(FormInvocationId);
id!(FormStreamId);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormPhase {
    Enter,
    Leave,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationOrigin {
    PageContent {
        span: Range<usize>,
    },
    FormContent {
        invocation: FormInvocationId,
        span: Range<usize>,
    },
    FormBoundary {
        invocation: FormInvocationId,
        phase: FormPhase,
    },
    /// Synthetic q/Q isolating page content from the annotations drawn after it.
    PageScope {
        phase: FormPhase,
    },
}
#[derive(Clone, Debug)]
pub struct ResourceScope {
    pub(crate) id: ResourceScopeId,
    pub(crate) provider: Option<ObjectId>,
    pub(crate) resource_object: Option<ObjectId>,
}
impl ResourceScope {
    pub fn id(&self) -> ResourceScopeId {
        self.id
    }
    pub fn provider(&self) -> Option<ObjectId> {
        self.provider
    }
    pub fn resource_object(&self) -> Option<ObjectId> {
        self.resource_object
    }
}
#[derive(Clone, Debug)]
pub struct FormStream {
    pub(crate) object: ObjectId,
    pub(crate) bytes: Arc<Vec<u8>>,
}
impl FormStream {
    pub fn object_id(&self) -> ObjectId {
        self.object
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
#[derive(Clone, Debug)]
pub struct FormInvocation {
    pub(crate) id: FormInvocationId,
    pub(crate) parent: Option<FormInvocationId>,
    pub(crate) stream: FormStreamId,
    pub(crate) object: ObjectId,
    pub(crate) caller_scope: ResourceScopeId,
    pub(crate) effective_scope: ResourceScopeId,
    pub(crate) call_origin: OperationOrigin,
    pub(crate) matrix: Affine<UserSpace, UserSpace>,
    pub(crate) bbox: Rect<UserSpace>,
    pub(crate) execution_range: Range<usize>,
    /// Annotation whose normal appearance this top-level invocation draws.
    pub(crate) annotation: Option<ObjectId>,
}
impl FormInvocation {
    pub fn annotation(&self) -> Option<ObjectId> {
        self.annotation
    }
    pub fn id(&self) -> FormInvocationId {
        self.id
    }
    pub fn parent(&self) -> Option<FormInvocationId> {
        self.parent
    }
    pub fn stream_id(&self) -> FormStreamId {
        self.stream
    }
    pub fn object_id(&self) -> ObjectId {
        self.object
    }
    pub fn caller_scope(&self) -> ResourceScopeId {
        self.caller_scope
    }
    pub fn effective_scope(&self) -> ResourceScopeId {
        self.effective_scope
    }
    pub fn call_origin(&self) -> &OperationOrigin {
        &self.call_origin
    }
    pub fn matrix(&self) -> Affine<UserSpace, UserSpace> {
        self.matrix
    }
    pub fn bbox(&self) -> Rect<UserSpace> {
        self.bbox
    }
    pub fn execution_range(&self) -> Range<usize> {
        self.execution_range.clone()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormDiagnosticFrame {
    pub object: ObjectId,
    pub span: Range<usize>,
    pub caller_object: ObjectId,
    pub call_span: Range<usize>,
}
macro_rules! ledger {
    ($($field:ident => $cap:ident = $value:expr),* $(,)?) => {
        #[derive(Clone,Copy,Debug,Eq,PartialEq)] pub struct FormLimits { $(pub $cap:usize,)* pub max_depth:usize, pub max_stream_decoded_bytes:usize }
        #[derive(Clone,Copy,Debug,Default,Eq,PartialEq)] pub struct FormUsage { $(pub $field:usize,)* }
        impl Default for FormLimits { fn default()->Self { Self { $($cap:$value,)* max_depth:32,max_stream_decoded_bytes:8<<20 } } }
        impl FormLimits {
            pub fn intersect(self, other:Self)->Self { Self {$($cap:self.$cap.min(other.$cap),)*max_depth:self.max_depth.min(other.max_depth),max_stream_decoded_bytes:self.max_stream_decoded_bytes.min(other.max_stream_decoded_bytes)} }
            pub fn remaining(self, used:FormUsage)->Result<Self,FormError> { Ok(Self {$($cap:self.$cap.checked_sub(used.$field).ok_or_else(||err("budget",FormErrorKind::Invariant("usage exceeds cap")))?,)*max_depth:self.max_depth,max_stream_decoded_bytes:self.max_stream_decoded_bytes}) }
            fn page_ceiling(self, page:Self, used:FormUsage)->Self { Self {$($cap:self.$cap.min(used.$field.saturating_add(page.$cap)),)*max_depth:self.max_depth.min(page.max_depth).min(64),max_stream_decoded_bytes:self.max_stream_decoded_bytes.min(page.max_stream_decoded_bytes)} }
        }
        impl FormUsage { pub fn accumulate(&mut self, other:Self)->Result<(),FormError> { let next=Self {$($field:self.$field.checked_add(other.$field).ok_or_else(||err("budget",FormErrorKind::Invariant("usage overflow")))?,)*};*self=next;Ok(()) } }
    }
}
ledger! {
    invocations=>max_invocations=100_000, unique_streams=>max_unique_streams=4096,
    resource_scopes=>max_resource_scopes=8192,resource_bindings=>max_resource_bindings=262_144,
    name_bytes=>max_name_bytes=16<<20,metadata_items=>max_metadata_items=1_000_000,
    reference_hops=>max_reference_hops=1_000_000,lookup_work=>max_lookup_work=100_000_000,
    origin_records=>max_origin_records=12_000_000,allocated_bytes=>max_allocated_bytes=256<<20,
    diagnostic_allocated_bytes=>max_diagnostic_allocated_bytes=64<<20,
}
#[derive(Debug)]
pub enum FormErrorKind {
    Document(Box<DocumentError>),
    Geometry(GeometryError),
    Invalid(&'static str),
    Unsupported(&'static str),
    Limit(&'static str),
    Cycle(ObjectId),
    Allocation,
    Invariant(&'static str),
}
#[derive(Debug)]
pub struct FormError {
    pub object: Option<ObjectId>,
    pub field: &'static str,
    pub kind: FormErrorKind,
}
impl fmt::Display for FormError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Form {:?} {}: {:?}", self.object, self.field, self.kind)
    }
}
impl std::error::Error for FormError {}
fn err(field: &'static str, kind: FormErrorKind) -> FormError {
    FormError {
        object: None,
        field,
        kind,
    }
}
fn charge(used: &mut usize, n: usize, cap: usize, label: &'static str) -> Result<(), FormError> {
    *used = used
        .checked_add(n)
        .filter(|v| *v <= cap)
        .ok_or_else(|| err(label, FormErrorKind::Limit(label)))?;
    Ok(())
}
#[derive(Clone, Copy)]
struct Resolved<'a> {
    value: &'a Object,
    id: Option<ObjectId>,
    stream: bool,
}
pub(crate) struct ScopedResources<'a> {
    pub descriptor: ResourceScope,
    pub resources: Option<&'a Dictionary>,
}
#[derive(Clone)]
pub(crate) struct FormDefinition {
    pub object: ObjectId,
    pub matrix: Affine<UserSpace, UserSpace>,
    pub bbox: Rect<UserSpace>,
    pub scope: Option<ResourceScopeId>,
    pub bytes: Option<Arc<Vec<u8>>>,
}
#[derive(Clone, Copy)]
pub(crate) enum XObjectDefinition {
    Form(usize),
    Image(ObjectId),
}
pub(crate) struct FormResourceLoader<'d, 'i> {
    document: &'d Document<'i>,
    limits: FormLimits,
    active: FormLimits,
    usage: FormUsage,
    scopes: Vec<ScopedResources<'d>>,
    forms: Vec<FormDefinition>,
}
impl<'d, 'i> FormResourceLoader<'d, 'i> {
    pub fn new(document: &'d Document<'i>, limits: FormLimits) -> Self {
        Self {
            document,
            limits,
            active: limits,
            usage: FormUsage::default(),
            scopes: Vec::new(),
            forms: Vec::new(),
        }
    }
    pub fn document(&self) -> &'d Document<'i> {
        self.document
    }
    pub fn begin_page(&mut self, limits: FormLimits) {
        self.active = self.limits.page_ceiling(limits, self.usage);
    }
    pub fn usage(&self) -> FormUsage {
        self.usage
    }
    pub fn active_limits(&self) -> FormLimits {
        self.active
    }
    pub fn work(&mut self, n: usize) -> Result<(), FormError> {
        charge(
            &mut self.usage.lookup_work,
            n,
            self.active.max_lookup_work,
            "lookup work",
        )
    }
    fn document_lookup(&mut self) -> Result<(), FormError> {
        // A lookup can compare both ObjectId scalars at every index entry.
        let work = self
            .document
            .object_count()
            .checked_add(1)
            .and_then(|n| n.checked_mul(2))
            .ok_or_else(|| err("lookup work", FormErrorKind::Limit("lookup arithmetic")))?;
        self.work(work)
    }
    pub fn allocation(&mut self, n: usize) -> Result<(), FormError> {
        charge(
            &mut self.usage.allocated_bytes,
            n,
            self.active.max_allocated_bytes,
            "allocation",
        )
    }
    pub fn origin(&mut self) -> Result<(), FormError> {
        charge(
            &mut self.usage.origin_records,
            1,
            self.active.max_origin_records,
            "origin records",
        )
    }
    pub fn invocation(&mut self) -> Result<(), FormError> {
        charge(
            &mut self.usage.invocations,
            1,
            self.active.max_invocations,
            "invocations",
        )
    }
    pub fn diagnostic(&mut self) -> Result<Vec<FormDiagnosticFrame>, FormError> {
        let n = 64usize
            .checked_mul(size_of::<FormDiagnosticFrame>())
            .ok_or_else(|| err("diagnostic", FormErrorKind::Limit("arithmetic")))?;
        charge(
            &mut self.usage.diagnostic_allocated_bytes,
            n,
            self.active.max_diagnostic_allocated_bytes,
            "diagnostic allocation",
        )?;
        let mut v = Vec::new();
        v.try_reserve_exact(64)
            .map_err(|_| err("diagnostic", FormErrorKind::Allocation))?;
        Ok(v)
    }
    pub fn reserve<T>(&mut self, v: &mut Vec<T>, max: usize) -> Result<(), FormError> {
        if v.len() >= max {
            return Err(err("output", FormErrorKind::Limit("output count")));
        }
        if v.len() < v.capacity() {
            return Ok(());
        }
        let n = v.capacity().saturating_mul(2).max(1).min(max);
        self.allocation(
            n.checked_mul(size_of::<T>())
                .ok_or_else(|| err("allocation", FormErrorKind::Limit("arithmetic")))?,
        )?;
        v.try_reserve_exact(n - v.len())
            .map_err(|_| err("allocation", FormErrorKind::Allocation))
    }
    fn resolve(
        &mut self,
        mut value: &'d Object,
        field: &'static str,
    ) -> Result<Resolved<'d>, FormError> {
        let mut seen = [None; 64];
        let mut count = 0;
        let mut id = None;
        let mut stream = false;
        while let Object::Reference(next) = value {
            self.work(count + 1)?;
            if seen[..count].contains(&Some(*next)) {
                return Err(err(field, FormErrorKind::Cycle(*next)));
            }
            if count == 64 {
                return Err(err(field, FormErrorKind::Limit("reference depth")));
            }
            charge(
                &mut self.usage.reference_hops,
                1,
                self.active.max_reference_hops,
                "reference hops",
            )?;
            seen[count] = Some(*next);
            count += 1;
            self.document_lookup()?;
            let object = self
                .document
                .object(*next)
                .map_err(|e| err(field, FormErrorKind::Document(Box::new(e))))?;
            id = Some(*next);
            stream = object.stream.is_some();
            value = &object.value;
        }
        Ok(Resolved { value, id, stream })
    }
    fn get(
        &mut self,
        d: &'d Dictionary,
        key: &'static [u8],
        field: &'static str,
    ) -> Result<Option<Resolved<'d>>, FormError> {
        charge(
            &mut self.usage.metadata_items,
            1,
            self.active.max_metadata_items,
            "metadata items",
        )?;
        self.work(
            d.len()
                .saturating_add(1)
                .checked_mul(key.len() + 1)
                .ok_or_else(|| err(field, FormErrorKind::Limit("lookup arithmetic")))?,
        )?;
        let Some(v) = d.get(key) else { return Ok(None) };
        let r = self.resolve(v, field)?;
        Ok((!matches!(r.value, Object::Null)).then_some(r))
    }
    fn dict(r: Resolved<'d>, field: &'static str) -> Result<&'d Dictionary, FormError> {
        if r.stream {
            return Err(err(
                field,
                FormErrorKind::Invalid("stream where dictionary required"),
            ));
        }
        match r.value {
            Object::Dictionary(d) => Ok(d),
            _ => Err(err(field, FormErrorKind::Invalid("expected dictionary"))),
        }
    }
    fn scope(
        &mut self,
        provider: Option<ObjectId>,
        resources: Option<Resolved<'d>>,
    ) -> Result<ResourceScopeId, FormError> {
        self.work(self.scopes.len())?;
        if let Some(index) = self
            .scopes
            .iter()
            .position(|s| s.descriptor.provider == provider)
        {
            return Ok(ResourceScopeId(index));
        }
        let d = resources.map(|r| Self::dict(r, "Resources")).transpose()?;
        if let Some(d) = d {
            if let Some(colors) = self.get(d, b"ColorSpace", "Resources.ColorSpace")? {
                let colors = Self::dict(colors, "Resources.ColorSpace")?;
                for key in [b"DefaultGray".as_slice(), b"DefaultRGB", b"DefaultCMYK"] {
                    if self
                        .get(colors, key, "Resources.ColorSpace.Default")?
                        .is_some()
                    {
                        return Err(err(
                            "Resources.ColorSpace",
                            FormErrorKind::Unsupported("default device color space"),
                        ));
                    }
                }
            }
            for (key, field) in [
                (b"Font".as_slice(), "Resources.Font"),
                (b"XObject".as_slice(), "Resources.XObject"),
            ] {
                if let Some(bindings) = self.get(d, key, field)? {
                    let bindings = Self::dict(bindings, field)?;
                    charge(
                        &mut self.usage.resource_bindings,
                        bindings.len(),
                        self.active.max_resource_bindings,
                        "resource bindings",
                    )?;
                    for name in bindings.keys() {
                        charge(
                            &mut self.usage.name_bytes,
                            name.len(),
                            self.active.max_name_bytes,
                            "resource name bytes",
                        )?;
                        self.work(name.len() + 1)?;
                    }
                }
            }
        }
        charge(
            &mut self.usage.resource_scopes,
            1,
            self.active.max_resource_scopes,
            "resource scopes",
        )?;
        let id = ResourceScopeId(self.scopes.len());
        let mut scopes = std::mem::take(&mut self.scopes);
        let result = self.reserve(&mut scopes, self.active.max_resource_scopes);
        self.scopes = scopes;
        result?;
        self.scopes.push(ScopedResources {
            descriptor: ResourceScope {
                id,
                provider,
                resource_object: resources.and_then(|r| r.id),
            },
            resources: d,
        });
        Ok(id)
    }
    pub fn root_scope(&mut self, page: ObjectId) -> Result<ResourceScopeId, FormError> {
        let mut current = page;
        let mut seen = [None; 64];
        for depth in 0..64 {
            self.work(depth + 1)?;
            if seen[..depth].contains(&Some(current)) {
                return Err(err("Parent", FormErrorKind::Cycle(current)));
            }
            seen[depth] = Some(current);
            charge(
                &mut self.usage.reference_hops,
                1,
                self.active.max_reference_hops,
                "reference hops",
            )?;
            self.document_lookup()?;
            let object = self
                .document
                .object(current)
                .map_err(|e| err("Page", FormErrorKind::Document(Box::new(e))))?;
            if object.stream.is_some() {
                return Err(err("Page", FormErrorKind::Invalid("page stream")));
            }
            let Object::Dictionary(d) = &object.value else {
                return Err(err("Page", FormErrorKind::Invalid("page dictionary")));
            };
            if let Some(resources) = self.get(d, b"Resources", "Resources")? {
                return self.scope(Some(current), Some(resources));
            }
            charge(
                &mut self.usage.metadata_items,
                1,
                self.active.max_metadata_items,
                "metadata items",
            )?;
            self.work(
                d.len()
                    .saturating_add(1)
                    .checked_mul(7)
                    .ok_or_else(|| err("Parent", FormErrorKind::Limit("lookup arithmetic")))?,
            )?;
            match d.get(b"Parent".as_slice()) {
                None | Some(Object::Null) => return self.scope(None, None),
                Some(Object::Reference(id)) => current = *id,
                _ => return Err(err("Parent", FormErrorKind::Invalid("parent reference"))),
            }
        }
        Err(err("Parent", FormErrorKind::Limit("parent depth")))
    }
    pub fn scoped(&self, id: ResourceScopeId) -> &ScopedResources<'d> {
        &self.scopes[id.0]
    }
    pub fn definition(&self, index: usize) -> &FormDefinition {
        &self.forms[index]
    }
    pub fn set_bytes(&mut self, index: usize, bytes: Arc<Vec<u8>>) {
        self.forms[index].bytes = Some(bytes);
    }
    pub fn lookup(
        &mut self,
        scope: ResourceScopeId,
        name: &[u8],
    ) -> Result<XObjectDefinition, FormError> {
        (|| {
            let d = self.scopes[scope.0].resources.ok_or_else(|| {
                err(
                    "Resources.XObject",
                    FormErrorKind::Invalid("missing XObject resource"),
                )
            })?;
            let objects = self
                .get(d, b"XObject", "Resources.XObject")?
                .ok_or_else(|| {
                    err(
                        "Resources.XObject",
                        FormErrorKind::Invalid("missing XObject dictionary"),
                    )
                })?;
            let objects = Self::dict(objects, "Resources.XObject")?;
            self.work(
                objects
                    .len()
                    .checked_mul(name.len().saturating_add(1))
                    .ok_or_else(|| err("lookup", FormErrorKind::Limit("arithmetic")))?,
            )?;
            let v = objects.get(name).ok_or_else(|| {
                err(
                    "Resources.XObject",
                    FormErrorKind::Invalid("missing XObject name"),
                )
            })?;
            let r = self.resolve(v, "XObject")?;
            self.define(r, false)
        })()
    }
    /// Define an XObject named by an indirect reference rather than a
    /// resource name, as annotation appearance streams are.
    pub fn lookup_object(&mut self, id: ObjectId) -> Result<XObjectDefinition, FormError> {
        self.work(1)?;
        charge(
            &mut self.usage.reference_hops,
            1,
            self.active.max_reference_hops,
            "reference hops",
        )?;
        self.document_lookup()?;
        let object = self
            .document
            .object(id)
            .map_err(|e| err("XObject", FormErrorKind::Document(Box::new(e))))?;
        self.define(
            Resolved {
                value: &object.value,
                id: Some(id),
                stream: object.stream.is_some(),
            },
            true,
        )
    }
    /// Annotation appearance streams (`appearance`) are Forms by context, so
    /// a missing /Subtype is accepted there as viewers do.
    fn define(
        &mut self,
        r: Resolved<'d>,
        appearance: bool,
    ) -> Result<XObjectDefinition, FormError> {
        (|| {
            let id = r.id.ok_or_else(|| {
                err(
                    "XObject",
                    FormErrorKind::Invalid("indirect stream required"),
                )
            })?;
            if !r.stream {
                return Err(err(
                    "XObject",
                    FormErrorKind::Invalid("XObject must be stream"),
                ));
            }
            self.work(self.forms.len())?;
            if let Some(i) = self.forms.iter().position(|f| f.object == id) {
                return Ok(XObjectDefinition::Form(i));
            }
            let Object::Dictionary(d) = r.value else {
                return Err(err("XObject", FormErrorKind::Invalid("stream dictionary")));
            };
            let checked = (|| {
                if let Some(t) = self.get(d, b"Type", "Type")? {
                    if t.stream || !matches!(t.value,Object::Name(n) if n==b"XObject") {
                        return Err(err("Type", FormErrorKind::Invalid("XObject type")));
                    }
                }
                match self.get(d, b"Subtype", "Subtype")? {
                    None if appearance => {}
                    None => return Err(err("Subtype", FormErrorKind::Invalid("missing subtype"))),
                    Some(subtype)
                        if !subtype.stream
                            && matches!(subtype.value,Object::Name(n) if n==b"Image") =>
                    {
                        return Ok(XObjectDefinition::Image(id));
                    }
                    Some(subtype)
                        if subtype.stream
                            || !matches!(subtype.value,Object::Name(n) if n==b"Form") =>
                    {
                        return Err(err(
                            "Subtype",
                            FormErrorKind::Unsupported("only ordinary Form XObjects"),
                        ));
                    }
                    Some(_) => {}
                }
                // /OC is evaluated by the page program, which hides the whole Form.
                for key in [b"Group".as_slice(), b"Ref", b"OPI", b"Subtype2"] {
                    if self.get(d, key, "Form semantics")?.is_some() {
                        return Err(err(
                            "Form semantics",
                            FormErrorKind::Unsupported(
                                "group/optional/reference/alternate Form semantics",
                            ),
                        ));
                    }
                }
                if let Some(t) = self.get(d, b"FormType", "FormType")? {
                    if t.stream || !matches!(t.value, Object::Integer(1)) {
                        return Err(err(
                            "FormType",
                            FormErrorKind::Unsupported("only FormType 1"),
                        ));
                    }
                }
                let bbox = self
                    .get(d, b"BBox", "BBox")?
                    .ok_or_else(|| err("BBox", FormErrorKind::Invalid("missing bounding box")))?;
                let b = self.numbers::<4>(bbox, "BBox")?;
                if b[0] >= b[2] || b[1] >= b[3] {
                    return Err(err(
                        "BBox",
                        FormErrorKind::Unsupported("empty or reversed bounding box"),
                    ));
                }
                let bbox = Rect::from_corners(b[0], b[1], b[2], b[3])
                    .map_err(|e| err("BBox", FormErrorKind::Geometry(e)))?;
                let matrix = if let Some(m) = self.get(d, b"Matrix", "Matrix")? {
                    Affine::from_pdf(self.numbers::<6>(m, "Matrix")?)
                        .map_err(|e| err("Matrix", FormErrorKind::Geometry(e)))?
                } else {
                    Affine::identity()
                };
                matrix
                    .inverse()
                    .map_err(|e| err("Matrix", FormErrorKind::Geometry(e)))?;
                let resources = self.get(d, b"Resources", "Resources")?;
                let explicit = resources
                    .map(|r| self.scope(Some(id), Some(r)))
                    .transpose()?;
                charge(
                    &mut self.usage.unique_streams,
                    1,
                    self.active.max_unique_streams,
                    "unique streams",
                )?;
                let mut forms = std::mem::take(&mut self.forms);
                let result = self.reserve(&mut forms, self.active.max_unique_streams);
                self.forms = forms;
                result?;
                let index = self.forms.len();
                self.forms.push(FormDefinition {
                    object: id,
                    matrix,
                    bbox,
                    scope: explicit,
                    bytes: None,
                });
                Ok(XObjectDefinition::Form(index))
            })();
            checked.map_err(|mut e| {
                e.object = Some(id);
                e
            })
        })()
    }
    /// Admit a conservative duplicate-metadata pass for the existing decoder.
    /// Its exact accepted grammar is shallow: Filter names and DecodeParms
    /// dictionaries/arrays; every reference/value it can visit is bounded here.
    pub fn meter_decoder(&mut self, index: usize) -> Result<(), FormError> {
        let before = self.usage;
        let id = self.forms[index].object;
        self.document_lookup()?;
        let object = self
            .document
            .object(id)
            .map_err(|e| err("stream", FormErrorKind::Document(Box::new(e))))?;
        let Object::Dictionary(d) = &object.value else {
            return Err(err("stream", FormErrorKind::Invalid("stream dictionary")));
        };
        for key in [b"Filter".as_slice(), b"DecodeParms", b"F"] {
            if let Some(value) = self.get(d, key, "stream metadata")? {
                match value.value {
                    Object::Array(a) => {
                        charge(
                            &mut self.usage.metadata_items,
                            a.len(),
                            self.active.max_metadata_items,
                            "metadata items",
                        )?;
                        for item in a {
                            let r = self.resolve(item, "stream metadata")?;
                            self.meter_parameter_dictionary(r)?;
                        }
                    }
                    _ => self.meter_parameter_dictionary(value)?,
                }
            }
        }
        // The dictionary-aware decoder visits this shallow structure again.
        let hops = self.usage.reference_hops - before.reference_hops;
        let items = self.usage.metadata_items - before.metadata_items;
        let work = self.usage.lookup_work - before.lookup_work;
        charge(
            &mut self.usage.reference_hops,
            hops,
            self.active.max_reference_hops,
            "reference hops",
        )?;
        charge(
            &mut self.usage.metadata_items,
            items,
            self.active.max_metadata_items,
            "metadata items",
        )?;
        self.work(work)?;
        // PP1 reads encoded length, and the decoder obtains the stream bytes.
        // Their separate index lookups are not part of the metadata duplicate.
        self.document_lookup()?;
        self.document_lookup()?;
        // Up to eight typed filter descriptors plus resolved parameter handles.
        self.allocation(16 * size_of::<crate::stream::FilterSpec>())?;
        Ok(())
    }
    fn meter_parameter_dictionary(&mut self, r: Resolved<'d>) -> Result<(), FormError> {
        if let Object::Name(name) = r.value {
            self.work(name.len().saturating_add(1).saturating_mul(8))?;
            self.allocation(name.len())?;
        }
        if let Object::Dictionary(d) = r.value {
            charge(
                &mut self.usage.metadata_items,
                d.len(),
                self.active.max_metadata_items,
                "metadata items",
            )?;
            for (key, value) in d {
                self.work(key.len().saturating_add(1).saturating_mul(10))?;
                self.resolve(value, "DecodeParms")?;
            }
        }
        Ok(())
    }
    fn numbers<const N: usize>(
        &mut self,
        r: Resolved<'d>,
        field: &'static str,
    ) -> Result<[f64; N], FormError> {
        if r.stream {
            return Err(err(field, FormErrorKind::Invalid("array stream")));
        }
        let Object::Array(a) = r.value else {
            return Err(err(field, FormErrorKind::Invalid("expected numeric array")));
        };
        if a.len() != N {
            return Err(err(field, FormErrorKind::Invalid("array length")));
        }
        let mut result = [0.; N];
        for (i, value) in a.iter().enumerate() {
            charge(
                &mut self.usage.metadata_items,
                1,
                self.active.max_metadata_items,
                "metadata items",
            )?;
            let r = self.resolve(value, field)?;
            if r.stream {
                return Err(err(field, FormErrorKind::Invalid("numeric stream")));
            }
            result[i] = match r.value {
                Object::Integer(n) => *n as f64,
                Object::Real(n) => *n,
                _ => return Err(err(field, FormErrorKind::Invalid("number required"))),
            };
            if !result[i].is_finite() {
                return Err(err(field, FormErrorKind::Invalid("nonfinite number")));
            }
        }
        Ok(result)
    }
}
