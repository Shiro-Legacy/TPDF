// SPDX-License-Identifier: AGPL-3.0-or-later
//! Immutable borrowed joins between Unicode policy and advances for one-byte
//! simple-font codes and two-byte Identity-H CID codes.
//! Registry names are owned once; font mappings and metrics are never cloned.
//! These bindings do not select glyph outlines or prove font-program identity.
use crate::builtin_metrics::{BuiltinFace, BuiltinMetricLookup, BuiltinMetrics};
use crate::cmap::ToUnicodeMap;
use crate::font_metrics::{AdvanceOrigin, FontAdvance, HorizontalMetrics};
use crate::simple_encoding::SimpleEncoding;
use crate::text_decode::{
    DecodedText, TextDecodeError, TextDecodeLimits, decode_encoded_text, decode_mapped_text,
};
use std::{error::Error, fmt, mem::size_of};

#[derive(Debug, Clone, Copy)]
pub enum UnicodeSource<'font> {
    ToUnicode(&'font ToUnicodeMap),
    Encoding(&'font SimpleEncoding),
}
#[derive(Debug, Clone, Copy)]
enum AdvanceSource<'font> {
    Declared(&'font HorizontalMetrics),
    Builtin(&'font BuiltinMetrics),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextUnicodeKind {
    ToUnicode,
    Encoding,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMetricKind {
    Declared,
    Builtin(BuiltinFace),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextMetricOrigin {
    Declared(AdvanceOrigin),
    Builtin(BuiltinFace),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFontLimitKind {
    Fonts,
    NameBytes,
    TotalNameBytes,
    CapacityBytes,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFontError {
    UnsupportedMapping,
    UndefinedBuiltinCode(u8),
    MissingBuiltinMetric(u8),
    DuplicateName,
    NameContainsNul,
    LimitExceeded(TextFontLimitKind),
    AllocationFailed,
}
impl fmt::Display for TextFontError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "text font binding error: {self:?}")
    }
}
impl Error for TextFontError {}

#[derive(Debug, Clone, Copy)]
pub struct TextFont<'font> {
    unicode: UnicodeSource<'font>,
    advances: AdvanceSource<'font>,
}
impl<'font> TextFont<'font> {
    pub fn declared(
        unicode: UnicodeSource<'font>,
        metrics: &'font HorizontalMetrics,
    ) -> Result<Self, TextFontError> {
        let width = if metrics.is_cid() { 2 } else { 1 };
        if metrics.is_cid() && !matches!(unicode, UnicodeSource::ToUnicode(_)) {
            return Err(TextFontError::UnsupportedMapping);
        }
        Self::validate_unicode(unicode, width)?;
        Ok(Self {
            unicode,
            advances: AdvanceSource::Declared(metrics),
        })
    }
    /// Bytes per character code: 2 for Identity-H CID fonts, else 1.
    pub fn code_bytes(self) -> usize {
        match self.advances {
            AdvanceSource::Declared(metrics) if metrics.is_cid() => 2,
            _ => 1,
        }
    }
    /// Use the metric encoding for Unicode unless an authoritative map is supplied.
    /// An error in that map never falls back to the metric encoding.
    pub fn builtin(
        metrics: &'font BuiltinMetrics,
        to_unicode: Option<&'font ToUnicodeMap>,
    ) -> Result<Self, TextFontError> {
        let unicode = to_unicode
            .map(UnicodeSource::ToUnicode)
            .unwrap_or(UnicodeSource::Encoding(metrics.encoding()));
        Self::validate_unicode(unicode, 1)?;
        Ok(Self {
            unicode,
            advances: AdvanceSource::Builtin(metrics),
        })
    }
    fn validate_unicode(source: UnicodeSource<'_>, width: usize) -> Result<(), TextFontError> {
        if let UnicodeSource::ToUnicode(map) = source {
            if map.writing_mode() != 0
                || map.codespaces().iter().any(|s| {
                    s.low().width().byte_len() != width || s.high().width().byte_len() != width
                })
                || map
                    .mappings()
                    .any(|(code, _)| code.width().byte_len() != width)
            {
                return Err(TextFontError::UnsupportedMapping);
            }
        }
        Ok(())
    }
    pub fn unicode_kind(self) -> TextUnicodeKind {
        match self.unicode {
            UnicodeSource::ToUnicode(_) => TextUnicodeKind::ToUnicode,
            UnicodeSource::Encoding(_) => TextUnicodeKind::Encoding,
        }
    }
    pub fn metric_kind(self) -> TextMetricKind {
        match self.advances {
            AdvanceSource::Declared(_) => TextMetricKind::Declared,
            AdvanceSource::Builtin(metrics) => TextMetricKind::Builtin(metrics.face()),
        }
    }
    pub fn decode<'input>(
        &self,
        input: &'input [u8],
        limits: TextDecodeLimits,
    ) -> Result<DecodedText<'input>, TextDecodeError> {
        match self.unicode {
            UnicodeSource::ToUnicode(map) => decode_mapped_text(map, input, limits),
            UnicodeSource::Encoding(encoding) => decode_encoded_text(encoding, input, limits),
        }
    }
    /// `code` is the whole character code; for Identity-H it is the CID.
    pub fn advance(self, code: u32) -> Result<(FontAdvance, TextMetricOrigin), TextFontError> {
        match self.advances {
            AdvanceSource::Declared(metrics) => {
                let value = if metrics.is_cid() {
                    metrics.lookup_cid(code)
                } else {
                    metrics
                        .lookup(u8::try_from(code).map_err(|_| TextFontError::UnsupportedMapping)?)
                };
                Ok((value.advance(), TextMetricOrigin::Declared(value.origin())))
            }
            AdvanceSource::Builtin(metrics) => {
                let code = u8::try_from(code).map_err(|_| TextFontError::UnsupportedMapping)?;
                match metrics.lookup(code) {
                    BuiltinMetricLookup::Advance(value) => {
                        Ok((value, TextMetricOrigin::Builtin(metrics.face())))
                    }
                    BuiltinMetricLookup::UndefinedGlyph => {
                        Err(TextFontError::UndefinedBuiltinCode(code))
                    }
                    BuiltinMetricLookup::MissingBuiltinMetric(_) => {
                        Err(TextFontError::MissingBuiltinMetric(code))
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextFontLimits {
    pub max_fonts: usize,
    pub max_name_bytes: usize,
    pub max_total_name_bytes: usize,
    pub max_capacity_bytes: usize,
}
impl Default for TextFontLimits {
    fn default() -> Self {
        Self {
            max_fonts: 256,
            max_name_bytes: 127,
            max_total_name_bytes: 32 * 1024,
            max_capacity_bytes: 64 * 1024,
        }
    }
}
/// Index scoped to one font registry and the trace produced from it.
/// IDs from different registries are not interchangeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextFontId(usize);
impl TextFontId {
    pub(crate) fn from_index(index: usize) -> Self {
        Self(index)
    }
    pub fn index(self) -> usize {
        self.0
    }
}
#[derive(Debug)]
struct Binding<'font> {
    name: Vec<u8>,
    font: TextFont<'font>,
}
#[derive(Debug)]
pub struct TextFonts<'font> {
    bindings: Vec<Binding<'font>>,
    capacity_bytes: usize,
    name_lookup: bool,
}
impl<'font> TextFonts<'font> {
    pub fn new(
        entries: &[(&[u8], TextFont<'font>)],
        limits: TextFontLimits,
    ) -> Result<Self, TextFontError> {
        Self::build(entries, limits, true)
    }
    /// PP1 has authenticated each scope/name binding; selection must use its ID.
    /// Duplicate display names never enable an ambiguous name-based fallback.
    pub(crate) fn new_indexed(
        entries: &[(&[u8], TextFont<'font>)],
        limits: TextFontLimits,
    ) -> Result<Self, TextFontError> {
        Self::build(entries, limits, false)
    }
    fn build(
        entries: &[(&[u8], TextFont<'font>)],
        limits: TextFontLimits,
        name_lookup: bool,
    ) -> Result<Self, TextFontError> {
        fn exceeded(kind: TextFontLimitKind) -> TextFontError {
            TextFontError::LimitExceeded(kind)
        }
        if entries.len() > limits.max_fonts {
            return Err(exceeded(TextFontLimitKind::Fonts));
        }
        let mut names = 0usize;
        for (index, (name, _)) in entries.iter().enumerate() {
            if name.len() > limits.max_name_bytes {
                return Err(exceeded(TextFontLimitKind::NameBytes));
            }
            if name.contains(&0) {
                return Err(TextFontError::NameContainsNul);
            }
            if name_lookup && entries[..index].iter().any(|(prior, _)| prior == name) {
                return Err(TextFontError::DuplicateName);
            }
            names = names
                .checked_add(name.len())
                .filter(|n| *n <= limits.max_total_name_bytes)
                .ok_or(exceeded(TextFontLimitKind::TotalNameBytes))?;
        }
        entries
            .len()
            .checked_mul(size_of::<Binding<'font>>())
            .and_then(|n| n.checked_add(names))
            .filter(|n| *n <= limits.max_capacity_bytes)
            .ok_or(exceeded(TextFontLimitKind::CapacityBytes))?;
        let mut bindings = Vec::new();
        bindings
            .try_reserve_exact(entries.len())
            .map_err(|_| TextFontError::AllocationFailed)?;
        let mut capacity_bytes = bindings
            .capacity()
            .checked_mul(size_of::<Binding<'font>>())
            .filter(|n| *n <= limits.max_capacity_bytes)
            .ok_or(exceeded(TextFontLimitKind::CapacityBytes))?;
        for (name, font) in entries {
            let mut owned_name = Vec::new();
            owned_name
                .try_reserve_exact(name.len())
                .map_err(|_| TextFontError::AllocationFailed)?;
            capacity_bytes = capacity_bytes
                .checked_add(owned_name.capacity())
                .filter(|n| *n <= limits.max_capacity_bytes)
                .ok_or(exceeded(TextFontLimitKind::CapacityBytes))?;
            owned_name.extend_from_slice(name);
            bindings.push(Binding {
                name: owned_name,
                font: *font,
            });
        }
        Ok(Self {
            bindings,
            capacity_bytes,
            name_lookup,
        })
    }
    pub fn len(&self) -> usize {
        self.bindings.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
    pub fn capacity_bytes(&self) -> usize {
        self.capacity_bytes
    }
    pub fn get(&self, name: &[u8]) -> Option<(TextFontId, &TextFont<'font>)> {
        if !self.name_lookup {
            return None;
        }
        self.bindings
            .iter()
            .enumerate()
            .find(|(_, binding)| binding.name == name)
            .map(|(index, binding)| (TextFontId(index), &binding.font))
    }
    pub fn by_id(&self, id: TextFontId) -> Option<&TextFont<'font>> {
        self.bindings.get(id.0).map(|binding| &binding.font)
    }
    pub fn name(&self, id: TextFontId) -> Option<&[u8]> {
        self.bindings
            .get(id.0)
            .map(|binding| binding.name.as_slice())
    }
}
