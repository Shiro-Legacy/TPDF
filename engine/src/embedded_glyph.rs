// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded PDF simple-TrueType and explicitly pinned Type1 outline binding.
//! Native policy references pinned MuPDF 1.28.1 pdf-font.c and fitz/font.c.
//! Only explicit format4 symbol/Unicode/MacRoman selections are supported.
//! Name-table and fuzzy glyph-name fallbacks are explicit per-code refusals.
//! Pinned Type1 binding retains exact byte Encoding independently of ToUnicode.
//! PDF loading, cmap validation/queries and outline work have separate budgets.
use crate::builtin_metrics::BuiltinFace;
use crate::document::{Document, DocumentError};
use crate::object::{Dictionary, Object, ObjectId};
use crate::simple_encoding::{
    EncodingError, EncodingLimits, GlyphUnicode, NamedEncoding, SimpleEncoding,
};
use crate::stream::StreamLimits;
use crate::stream_object::{StreamObjectError, decode_document_stream_with_usage};
use crate::truetype_cmap::{
    CmapError, CmapLimits, CmapLookup, CmapSelection, CmapUsage, TrueTypeCmapSession,
};
use crate::truetype_outline::{
    FontLimits, GlyphId, GlyphOutline, OutlineError, OutlineSession, TrueTypeFont,
};
use std::fmt;

/// Outline substitution is independent of the page's advance-metric policy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PageOutlinePolicy {
    #[default]
    EmbeddedOnly,
    AllowPinnedBase14,
    /// Display-only exact-name Arial/Helvetica bold style substitutions. PDF
    /// widths still control advances. Does not authorize portable text editing.
    AllowPinnedSubstitutes,
}
#[derive(Debug)]
pub enum LoadedGlyphFont {
    EmbeddedTrueType(EmbeddedGlyphFont),
    PinnedBase14(LoadedBuiltinGlyphFont),
}
#[derive(Debug)]
pub struct LoadedBuiltinGlyphFont {
    font_id: ObjectId,
    face: BuiltinFace,
    encoding: SimpleEncoding,
}
impl LoadedBuiltinGlyphFont {
    pub fn font_id(&self) -> ObjectId {
        self.font_id
    }
    pub fn face(&self) -> BuiltinFace {
        self.face
    }
    pub fn glyph_name(&self, code: u8) -> Option<&[u8]> {
        self.encoding.glyph_name(code)
    }
}
#[derive(Debug)]
pub enum GlyphError {
    Document(Box<DocumentError>),
    Stream(Box<StreamObjectError>),
    Encoding(EncodingError),
    Font(OutlineError),
    Cmap(CmapError),
    Invalid(&'static str),
    Unsupported(&'static str),
    Limit(&'static str),
    Missing(u32),
    UnsupportedCode(u32),
}
impl fmt::Display for GlyphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "embedded glyph: {self:?}")
    }
}
impl std::error::Error for GlyphError {}
impl From<OutlineError> for GlyphError {
    fn from(e: OutlineError) -> Self {
        Self::Font(e)
    }
}
impl From<CmapError> for GlyphError {
    fn from(e: CmapError) -> Self {
        Self::Cmap(e)
    }
}
#[derive(Debug, Clone, Copy)]
pub struct GlyphLoadLimits {
    pub max_attempts: usize,
    pub max_reference_hops: usize,
    pub max_metadata_items: usize,
    pub max_encoded_bytes: usize,
    pub max_decoded_work_bytes: usize,
    pub max_name_bytes: usize,
    pub stream: StreamLimits,
    pub encoding: EncodingLimits,
}
impl Default for GlyphLoadLimits {
    fn default() -> Self {
        Self {
            max_attempts: 128,
            max_reference_hops: 8192,
            max_metadata_items: 65536,
            max_encoded_bytes: 64 << 20,
            max_decoded_work_bytes: 128 << 20,
            max_name_bytes: 1 << 20,
            stream: StreamLimits::default(),
            encoding: EncodingLimits::default(),
        }
    }
}
#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub struct GlyphLoadUsage {
    pub attempts: usize,
    pub reference_hops: usize,
    pub metadata_items: usize,
    pub encoded_bytes: usize,
    pub decoded_work_bytes: usize,
    pub name_bytes: usize,
}
fn charge(
    value: &mut usize,
    amount: usize,
    max: usize,
    label: &'static str,
) -> Result<(), GlyphError> {
    *value = value
        .checked_add(amount)
        .filter(|n| *n <= max)
        .ok_or(GlyphError::Limit(label))?;
    Ok(())
}
struct Resolved<'a> {
    value: &'a Object,
    id: Option<ObjectId>,
    stream: bool,
}
fn dictionary<'a>(r: Resolved<'a>) -> Result<&'a Dictionary, GlyphError> {
    match r.value {
        Object::Dictionary(d) if !r.stream => Ok(d),
        _ => Err(GlyphError::Invalid("dictionary required, not a stream")),
    }
}

pub struct GlyphFontLoader<'doc, 'input> {
    document: &'doc Document<'input>,
    limits: GlyphLoadLimits,
    usage: GlyphLoadUsage,
}
impl<'doc, 'input> GlyphFontLoader<'doc, 'input> {
    pub fn new(document: &'doc Document<'input>, limits: GlyphLoadLimits) -> Self {
        Self {
            document,
            limits,
            usage: GlyphLoadUsage::default(),
        }
    }
    pub fn usage(&self) -> GlyphLoadUsage {
        self.usage
    }
    fn resolve(&mut self, mut value: &'doc Object) -> Result<Resolved<'doc>, GlyphError> {
        let mut ids = [None; 64];
        let mut n = 0;
        let mut last = None;
        let mut stream = false;
        while let Object::Reference(id) = value {
            if n == 64 || ids[..n].contains(&Some(*id)) {
                return Err(GlyphError::Invalid("reference cycle or depth"));
            }
            charge(
                &mut self.usage.reference_hops,
                1,
                self.limits.max_reference_hops,
                "reference hops",
            )?;
            ids[n] = Some(*id);
            n += 1;
            let object = self
                .document
                .object(*id)
                .map_err(|e| GlyphError::Document(Box::new(e)))?;
            value = &object.value;
            last = Some(*id);
            stream = object.stream.is_some();
            if stream {
                break;
            }
        }
        Ok(Resolved {
            value,
            id: last,
            stream,
        })
    }
    fn field(
        &mut self,
        dict: &'doc Dictionary,
        key: &[u8],
    ) -> Result<Option<Resolved<'doc>>, GlyphError> {
        charge(
            &mut self.usage.metadata_items,
            1,
            self.limits.max_metadata_items,
            "metadata items",
        )?;
        let Some(value) = dict.get(key) else {
            return Ok(None);
        };
        let r = self.resolve(value)?;
        if matches!(r.value, Object::Null) && !r.stream {
            Ok(None)
        } else {
            Ok(Some(r))
        }
    }
    fn check_type(&mut self, dict: &'doc Dictionary, expected: &[u8]) -> Result<(), GlyphError> {
        if let Some(value) = self.field(dict, b"Type")? {
            if value.stream || !matches!(value.value,Object::Name(name) if name==expected) {
                return Err(GlyphError::Invalid("contradictory dictionary Type"));
            }
        }
        Ok(())
    }
    fn encoding(&mut self, value: Resolved<'doc>) -> Result<SimpleEncoding, GlyphError> {
        if value.stream {
            return Err(GlyphError::Invalid("Encoding stream"));
        }
        let (base, differences) = match value.value {
            Object::Name(name) => (NamedEncoding::from_pdf_name(name), None),
            Object::Dictionary(dict) => {
                let base = self
                    .field(dict, b"BaseEncoding")?
                    .ok_or(GlyphError::Unsupported("implicit BaseEncoding"))?;
                let Object::Name(name) = base.value else {
                    return Err(GlyphError::Invalid("BaseEncoding name"));
                };
                if base.stream {
                    return Err(GlyphError::Invalid("BaseEncoding stream"));
                }
                (
                    NamedEncoding::from_pdf_name(name),
                    self.field(dict, b"Differences")?,
                )
            }
            _ => return Err(GlyphError::Invalid("Encoding name or dictionary")),
        };
        let encoding =
            SimpleEncoding::from_named(base.ok_or(GlyphError::Unsupported("named Encoding"))?);
        let Some(differences) = differences else {
            return Ok(encoding);
        };
        if differences.stream {
            return Err(GlyphError::Invalid("Differences stream"));
        }
        let Object::Array(items) = differences.value else {
            return Err(GlyphError::Invalid("Differences array"));
        };
        if items.len() > self.limits.encoding.max_items {
            return Err(GlyphError::Limit("Differences items"));
        }
        charge(
            &mut self.usage.metadata_items,
            items.len(),
            self.limits.max_metadata_items,
            "Differences items",
        )?;
        // Name bytes charge before S0 allocates its final256 slots; failures retain work.
        for item in items {
            if let Object::Name(name) = item {
                charge(
                    &mut self.usage.name_bytes,
                    name.len(),
                    self.limits.max_name_bytes,
                    "encoding name bytes",
                )?;
            }
        }
        encoding
            .with_object_differences(differences.value, self.limits.encoding)
            .map_err(GlyphError::Encoding)
    }
    pub fn load(&mut self, font_id: ObjectId) -> Result<EmbeddedGlyphFont, GlyphError> {
        match self.load_source(font_id, PageOutlinePolicy::EmbeddedOnly)? {
            LoadedGlyphFont::EmbeddedTrueType(font) => Ok(font),
            LoadedGlyphFont::PinnedBase14(_) => unreachable!("embedded-only dispatch"),
        }
    }
    pub fn load_source(
        &mut self,
        font_id: ObjectId,
        policy: PageOutlinePolicy,
    ) -> Result<LoadedGlyphFont, GlyphError> {
        charge(
            &mut self.usage.attempts,
            1,
            self.limits.max_attempts,
            "font attempts",
        )?;
        charge(
            &mut self.usage.reference_hops,
            1,
            self.limits.max_reference_hops,
            "reference hops",
        )?;
        let object = self
            .document
            .object(font_id)
            .map_err(|e| GlyphError::Document(Box::new(e)))?;
        if object.stream.is_some() {
            return Err(GlyphError::Invalid("font object stream"));
        }
        let font = dictionary(self.resolve(&object.value)?)?;
        self.check_type(font, b"Font")?;
        let subtype = self
            .field(font, b"Subtype")?
            .ok_or(GlyphError::Invalid("missing Subtype"))?;
        if !subtype.stream
            && matches!(subtype.value,Object::Name(n) if n==b"Type1")
            && policy != PageOutlinePolicy::EmbeddedOnly
        {
            return self
                .load_builtin(font_id, font, policy)
                .map(LoadedGlyphFont::PinnedBase14);
        }
        if !subtype.stream
            && matches!(subtype.value,Object::Name(n) if n==b"TrueType")
            && policy == PageOutlinePolicy::AllowPinnedSubstitutes
        {
            // Embedded TrueType always wins. Only known names with no embedded
            // program may enter the substitute path; malformed descriptors fail.
            if let Some(descriptor) = self.field(font, b"FontDescriptor")? {
                let descriptor = dictionary(descriptor)?;
                let embedded = [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
                    .iter()
                    .try_fold(false, |found, key| -> Result<bool, GlyphError> {
                        Ok(found || self.field(descriptor, key)?.is_some())
                    })?;
                if !embedded {
                    return self
                        .load_builtin(font_id, font, policy)
                        .map(LoadedGlyphFont::PinnedBase14);
                }
            }
        }
        if !subtype.stream && matches!(subtype.value,Object::Name(n) if n==b"Type0") {
            return self.load_cid(font_id, font);
        }
        if subtype.stream || !matches!(subtype.value,Object::Name(n) if n==b"TrueType") {
            return Err(GlyphError::Unsupported("only embedded simple TrueType"));
        }
        let descriptor = dictionary(
            self.field(font, b"FontDescriptor")?
                .ok_or(GlyphError::Invalid("missing FontDescriptor"))?,
        )?;
        self.check_type(descriptor, b"FontDescriptor")?;
        let flags = self
            .field(descriptor, b"Flags")?
            .ok_or(GlyphError::Invalid("missing Flags"))?;
        let flags = match flags.value {
            Object::Integer(v) if !flags.stream => {
                u32::try_from(*v).map_err(|_| GlyphError::Invalid("Flags integer range"))?
            }
            _ => return Err(GlyphError::Invalid("Flags integer")),
        };
        let program_id = self.font_file2(descriptor)?;
        let symbolic = flags & 4 != 0 && flags & 32 == 0;
        let encoding = self.field(font, b"Encoding")?;
        let encoding = if symbolic {
            if encoding.is_some() {
                return Err(GlyphError::Unsupported("symbolic with explicit Encoding"));
            }
            None
        } else {
            Some(self.encoding(
                encoding.ok_or(GlyphError::Unsupported("non-symbolic implicit Encoding"))?,
            )?)
        };
        Ok(LoadedGlyphFont::EmbeddedTrueType(EmbeddedGlyphFont {
            font_id,
            program_id,
            flags,
            encoding,
            cid_to_gid: None,
            bytes: self.decode(program_id)?,
        }))
    }
    fn font_file2(&mut self, descriptor: &'doc Dictionary) -> Result<ObjectId, GlyphError> {
        for key in [b"FontFile".as_slice(), b"FontFile3"] {
            if self.field(descriptor, key)?.is_some() {
                return Err(GlyphError::Unsupported("alternate embedded program"));
            }
        }
        let program = self
            .field(descriptor, b"FontFile2")?
            .ok_or(GlyphError::Invalid("missing FontFile2"))?;
        if !program.stream {
            return Err(GlyphError::Invalid("FontFile2 requires an indirect stream"));
        }
        program.id.ok_or(GlyphError::Invalid("program identity"))
    }
    /// Identity-H Type0 with one CIDFontType2 descendant. PP1 has validated the
    /// encoding and widths; this binds the embedded program and CIDToGIDMap.
    fn load_cid(
        &mut self,
        font_id: ObjectId,
        font: &'doc Dictionary,
    ) -> Result<LoadedGlyphFont, GlyphError> {
        let descendants = self
            .field(font, b"DescendantFonts")?
            .ok_or(GlyphError::Invalid("missing DescendantFonts"))?;
        let descendant = match descendants.value {
            Object::Array(items) if !descendants.stream && items.len() == 1 => {
                dictionary(self.resolve(&items[0])?)?
            }
            _ => return Err(GlyphError::Invalid("DescendantFonts array")),
        };
        self.check_type(descendant, b"Font")?;
        let subtype = self
            .field(descendant, b"Subtype")?
            .ok_or(GlyphError::Invalid("missing CIDFont Subtype"))?;
        if subtype.stream || !matches!(subtype.value,Object::Name(n) if n==b"CIDFontType2") {
            return Err(GlyphError::Unsupported("only CIDFontType2 descendants"));
        }
        let descriptor = dictionary(
            self.field(descendant, b"FontDescriptor")?
                .ok_or(GlyphError::Invalid("missing FontDescriptor"))?,
        )?;
        self.check_type(descriptor, b"FontDescriptor")?;
        let program_id = self.font_file2(descriptor)?;
        let cid_to_gid = match self.field(descendant, b"CIDToGIDMap")? {
            None => CidToGid::Identity,
            Some(map)
                if !map.stream && matches!(map.value, Object::Name(n) if n == b"Identity") =>
            {
                CidToGid::Identity
            }
            Some(map) if map.stream => {
                let bytes = self.decode(map.id.ok_or(GlyphError::Invalid("map identity"))?)?;
                if bytes.len() % 2 != 0 {
                    return Err(GlyphError::Invalid("odd CIDToGIDMap length"));
                }
                CidToGid::Table(
                    bytes
                        .chunks_exact(2)
                        .map(|b| u16::from_be_bytes([b[0], b[1]]))
                        .collect(),
                )
            }
            Some(_) => return Err(GlyphError::Invalid("CIDToGIDMap name or stream")),
        };
        Ok(LoadedGlyphFont::EmbeddedTrueType(EmbeddedGlyphFont {
            font_id,
            program_id,
            flags: 0,
            encoding: None,
            cid_to_gid: Some(cid_to_gid),
            bytes: self.decode(program_id)?,
        }))
    }
    fn decode(&mut self, program_id: ObjectId) -> Result<Vec<u8>, GlyphError> {
        let mut stream = self.limits.stream;
        stream.max_encoded_bytes = stream
            .max_encoded_bytes
            .min(self.limits.max_encoded_bytes - self.usage.encoded_bytes);
        stream.max_total_decoded_bytes = stream
            .max_total_decoded_bytes
            .min(self.limits.max_decoded_work_bytes - self.usage.decoded_work_bytes);
        let encoded = stream.max_encoded_bytes;
        let decoded = stream.max_total_decoded_bytes;
        charge(
            &mut self.usage.encoded_bytes,
            encoded,
            self.limits.max_encoded_bytes,
            "encoded font bytes",
        )?;
        charge(
            &mut self.usage.decoded_work_bytes,
            decoded,
            self.limits.max_decoded_work_bytes,
            "font decode work",
        )?;
        let data = decode_document_stream_with_usage(self.document, program_id, stream)
            .map_err(|e| GlyphError::Stream(Box::new(e)))?;
        self.usage.encoded_bytes -= encoded - data.usage.encoded_bytes;
        self.usage.decoded_work_bytes -= decoded - data.usage.total_decoded_bytes;
        Ok(data.bytes)
    }
    fn load_builtin(
        &mut self,
        font_id: ObjectId,
        font: &'doc Dictionary,
        policy: PageOutlinePolicy,
    ) -> Result<LoadedBuiltinGlyphFont, GlyphError> {
        let base = self
            .field(font, b"BaseFont")?
            .ok_or(GlyphError::Invalid("missing BaseFont"))?;
        let face = match base.value {
            Object::Name(name) if !base.stream => {
                let face = if policy == PageOutlinePolicy::AllowPinnedSubstitutes {
                    BuiltinFace::from_substitute_name(name)
                } else {
                    BuiltinFace::from_pdf_name(name)
                };
                face.ok_or(GlyphError::Unsupported(
                    "font name outside pinned outline allowlist",
                ))?
            }
            _ => return Err(GlyphError::Invalid("BaseFont name")),
        };
        if let Some(descriptor) = self.field(font, b"FontDescriptor")? {
            let descriptor = dictionary(descriptor)?;
            self.check_type(descriptor, b"FontDescriptor")?;
            for key in [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"] {
                if self.field(descriptor, key)?.is_some() {
                    return Err(GlyphError::Unsupported(
                        "embedded program excludes pinned outlines",
                    ));
                }
            }
        }
        // This independent capability reads byte Encoding even when ToUnicode
        // authoritatively supplies text. PDF widths remain PP1's responsibility.
        let value = self
            .field(font, b"Encoding")?
            .ok_or(GlyphError::Unsupported(
                "pinned font requires explicit Encoding",
            ))?;
        let encoding = self.encoding(value)?;
        Ok(LoadedBuiltinGlyphFont {
            font_id,
            face,
            encoding,
        })
    }
}
#[derive(Debug)]
pub struct EmbeddedGlyphFont {
    font_id: ObjectId,
    program_id: ObjectId,
    flags: u32,
    encoding: Option<SimpleEncoding>,
    cid_to_gid: Option<CidToGid>,
    bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CidToGid {
    Identity,
    Table(Vec<u16>),
}
impl EmbeddedGlyphFont {
    /// Some for CIDFontType2 programs, which select glyphs by CID, not cmap.
    pub fn cid_to_gid(&self) -> Option<&CidToGid> {
        self.cid_to_gid.as_ref()
    }
    /// The GID for a CID. CIDs beyond a CIDToGIDMap table map to GID 0, which
    /// PP2 reports as missing rather than drawing .notdef.
    pub fn cid_glyph(&self, cid: u32) -> Option<u16> {
        let gid = match self.cid_to_gid.as_ref()? {
            CidToGid::Identity => u16::try_from(cid).ok()?,
            CidToGid::Table(table) => usize::try_from(cid)
                .ok()
                .and_then(|i| table.get(i).copied())?,
        };
        (gid != 0).then_some(gid)
    }
    pub fn font_id(&self) -> ObjectId {
        self.font_id
    }
    pub fn program_id(&self) -> ObjectId {
        self.program_id
    }
    pub fn flags(&self) -> u32 {
        self.flags
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn open(&self, limits: FontLimits) -> Result<TrueTypeFont<'_>, GlyphError> {
        Ok(TrueTypeFont::open(&self.bytes, limits)?)
    }
    /// The font is opened here from these owned bytes; callers cannot pair the
    /// PDF policy with a different font's validated directory.
    pub fn map(
        &self,
        font_limits: FontLimits,
        cmap_limits: CmapLimits,
    ) -> Result<SimpleGlyphMap, GlyphError> {
        let font = self.open(font_limits)?;
        map_font(&font, self.flags, self.encoding.as_ref(), cmap_limits)
    }
}
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum GlyphRoute {
    RawByte,
    PrivateUse,
    EncodingUnicode,
    EncodingMacRoman,
    Missing,
    UnsupportedNameFallback,
}
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct GlyphChoice {
    source: u8,
    gid: Option<GlyphId>,
    route: GlyphRoute,
    lookup_code: Option<u16>,
}
impl GlyphChoice {
    pub fn source(self) -> u8 {
        self.source
    }
    pub fn glyph_id(self) -> Option<GlyphId> {
        self.gid
    }
    pub fn route(self) -> GlyphRoute {
        self.route
    }
    pub fn lookup_code(self) -> Option<u16> {
        self.lookup_code
    }
}
#[derive(Debug)]
pub struct SimpleGlyphMap {
    selection: CmapSelection,
    choices: [GlyphChoice; 256],
    usage: CmapUsage,
}
impl SimpleGlyphMap {
    pub fn selection(&self) -> &CmapSelection {
        &self.selection
    }
    pub fn choices(&self) -> &[GlyphChoice; 256] {
        &self.choices
    }
    pub fn choice(&self, code: u8) -> GlyphChoice {
        self.choices[usize::from(code)]
    }
    pub fn usage(&self) -> CmapUsage {
        self.usage
    }
    /// Decode only through a caller-owned O0 session; its GID range check is
    /// repeated. Map/font identity must be bound by EmbeddedGlyphFont::outlines.
    fn decode(
        &self,
        code: u8,
        session: &mut OutlineSession<'_, '_>,
    ) -> Result<GlyphOutline, GlyphError> {
        let choice = self.choice(code);
        if choice.route == GlyphRoute::UnsupportedNameFallback {
            return Err(GlyphError::UnsupportedCode(code.into()));
        }
        let id = choice.gid.ok_or(GlyphError::Missing(code.into()))?;
        Ok(session.decode(id)?)
    }
}
/// Own the validated font, retaining the PDF map's identity without exposing a
/// constructor that pairs unrelated font bytes and glyph maps.
pub struct EmbeddedOutlines<'a> {
    font: TrueTypeFont<'a>,
    map: SimpleGlyphMap,
}
impl EmbeddedGlyphFont {
    pub fn outlines(
        &self,
        font_limits: FontLimits,
        cmap_limits: CmapLimits,
    ) -> Result<EmbeddedOutlines<'_>, GlyphError> {
        let font = self.open(font_limits)?;
        let map = map_font(&font, self.flags, self.encoding.as_ref(), cmap_limits)?;
        Ok(EmbeddedOutlines { font, map })
    }
}
pub struct GlyphOutlineSession<'font, 'bytes> {
    map: &'font SimpleGlyphMap,
    session: OutlineSession<'font, 'bytes>,
    code_attempts: usize,
    max_code_attempts: usize,
}
impl EmbeddedOutlines<'_> {
    pub fn map(&self) -> &SimpleGlyphMap {
        &self.map
    }
    pub fn font_usage(&self) -> crate::truetype_outline::FontUsage {
        self.font.usage()
    }
    pub fn units_per_em(&self) -> u16 {
        self.font.units_per_em()
    }
    pub fn glyph_count(&self) -> u16 {
        self.font.glyph_count()
    }
    pub fn session(
        &self,
        limits: crate::truetype_outline::OutlineLimits,
    ) -> GlyphOutlineSession<'_, '_> {
        GlyphOutlineSession {
            map: &self.map,
            session: self.font.session(limits),
            code_attempts: 0,
            max_code_attempts: limits.max_decode_attempts,
        }
    }
}
impl GlyphOutlineSession<'_, '_> {
    pub fn decode(&mut self, code: u8) -> Result<GlyphOutline, GlyphError> {
        charge(
            &mut self.code_attempts,
            1,
            self.max_code_attempts,
            "code attempts",
        )?;
        self.map.decode(code, &mut self.session)
    }
    pub fn code_attempts(&self) -> usize {
        self.code_attempts
    }
    pub fn usage(&self) -> crate::truetype_outline::OutlineUsage {
        self.session.usage()
    }
}
fn lookup(
    cmap: &mut TrueTypeCmapSession<'_, '_>,
    code: u16,
) -> Result<(Option<GlyphId>, u16), GlyphError> {
    if let CmapLookup::Glyph(id) = cmap.lookup(code)? {
        return Ok((Some(id), code));
    }
    if let Some(pua) = code.checked_add(0xf000) {
        if let CmapLookup::Glyph(id) = cmap.lookup(pua)? {
            return Ok((Some(id), pua));
        }
    }
    if code == 0x22ef {
        if let CmapLookup::Glyph(id) = cmap.lookup(0x2026)? {
            return Ok((Some(id), 0x2026));
        }
    }
    Ok((None, code))
}
fn mac_code(name: &[u8], encoding: &SimpleEncoding) -> Option<u16> {
    // Pinned MuPDF lookup_mre_code corrections precede first-name MacRoman lookup.
    const FIXES: [(u16, &[u8]); 16] = [
        (173, b"notequal"),
        (176, b"infinity"),
        (178, b"lessequal"),
        (179, b"greaterequal"),
        (182, b"partialdiff"),
        (183, b"summation"),
        (184, b"product"),
        (185, b"pi"),
        (186, b"integral"),
        (189, b"Omega"),
        (195, b"radical"),
        (197, b"approxequal"),
        (198, b"Delta"),
        (215, b"lozenge"),
        (219, b"Euro"),
        (240, b"apple"),
    ];
    if let Some((code, _)) = FIXES.iter().find(|(_, n)| *n == name) {
        return Some(*code);
    }
    (0..=255u16).find(|c| encoding.glyph_name(*c as u8) == Some(name))
}
fn map_font(
    font: &TrueTypeFont<'_>,
    flags: u32,
    encoding: Option<&SimpleEncoding>,
    limits: CmapLimits,
) -> Result<SimpleGlyphMap, GlyphError> {
    let symbolic = flags & 4 != 0 && flags & 32 == 0;
    let mut cmap = TrueTypeCmapSession::new(font, limits)?;
    let selectors: &[(u16, u16)] = if symbolic {
        &[(3, 0)]
    } else {
        &[(3, 1), (1, 0)]
    };
    let mut chosen = None;
    for &(platform, encoding) in selectors {
        let mut found = cmap
            .records()
            .filter(|r| r.platform_id() == platform && r.encoding_id() == encoding);
        if let Some(record) = found.next() {
            if found.next().is_some() {
                return Err(GlyphError::Unsupported("ambiguous cmap selector"));
            }
            chosen = Some(record);
            break;
        }
    }
    let record = chosen.ok_or(GlyphError::Unsupported("cmap platform policy"))?;
    let selection = cmap.select(record.index())?;
    let post_names = font
        .table_data(*b"post")
        .is_some_and(|bytes| bytes.get(..4) != Some(&[0, 3, 0, 0]));
    let empty = GlyphChoice {
        source: 0,
        gid: None,
        route: GlyphRoute::Missing,
        lookup_code: None,
    };
    let mut choices = [empty; 256];
    let mac_encoding = SimpleEncoding::from_named(NamedEncoding::MacRoman);
    for (index, choice) in choices.iter_mut().enumerate() {
        let code = index as u8;
        let (seed, query) = lookup(&mut cmap, u16::from(code))?;
        *choice = GlyphChoice {
            source: code,
            gid: seed,
            route: if seed.is_none() {
                GlyphRoute::Missing
            } else if query == u16::from(code) {
                GlyphRoute::RawByte
            } else {
                GlyphRoute::PrivateUse
            },
            lookup_code: Some(query),
        };
        if symbolic {
            continue;
        }
        let encoding = encoding.ok_or(GlyphError::Invalid("missing explicit encoding"))?;
        let Some(name) = encoding.glyph_name(code) else {
            continue;
        };
        if name == b".notdef" {
            continue;
        }
        let candidate = if record.platform_id() == 3 {
            match encoding.lookup_unicode(code) {
                GlyphUnicode::Unicode(chars) if chars.len() == 1 && u32::from(chars[0]) > 0 => {
                    u16::try_from(u32::from(chars[0])).ok()
                }
                _ => None,
            }
        } else {
            mac_code(name, &mac_encoding).filter(|v| *v > 0)
        };
        if let Some(candidate) = candidate {
            let (gid, query) = lookup(&mut cmap, candidate)?;
            if gid.is_some() {
                *choice = GlyphChoice {
                    source: code,
                    gid,
                    route: if record.platform_id() == 3 {
                        GlyphRoute::EncodingUnicode
                    } else {
                        GlyphRoute::EncodingMacRoman
                    },
                    lookup_code: Some(query),
                };
                continue;
            }
        }
        // Unsupported names can invoke native fuzzy Unicode or name-table
        // aliases. Never substitute raw-byte results when that branch is unknown.
        if post_names || (record.platform_id() == 3 && candidate.is_none()) {
            *choice = GlyphChoice {
                source: code,
                gid: None,
                route: GlyphRoute::UnsupportedNameFallback,
                lookup_code: None,
            };
        }
    }
    Ok(SimpleGlyphMap {
        selection,
        choices,
        usage: cmap.usage(),
    })
}

impl EmbeddedOutlines<'_> {
    /// Borrow the horizontal metrics from this same immutable font. Table
    /// validation is constant-sized and does not substitute PDF advance widths.
    pub fn horizontal_metrics(
        &self,
    ) -> Result<crate::truetype_metrics::HorizontalMetrics<'_>, crate::truetype_metrics::MetricsError>
    {
        crate::truetype_metrics::HorizontalMetrics::open(&self.font)
    }
}
