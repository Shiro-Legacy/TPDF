// SPDX-License-Identifier: AGPL-3.0-or-later
// PDF table behavior/data derived from pinned MuPDF 1.28.1, Artifex 2004-2021.
//! Explicit simple-font byte encodings. No font loading, implicit base encoding,
//! broad glyph-name heuristics, text layout, or automatic ToUnicode fallback.
//! Unicode lookup recognizes only the 378-name union of the four named tables.

use std::borrow::Cow;
use std::fmt;

use crate::object::Object;
#[path = "simple_encoding_data.rs"]
mod data;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamedEncoding {
    WinAnsi,
    MacRoman,
    Standard,
    MacExpert,
}
impl NamedEncoding {
    pub fn from_pdf_name(name: &[u8]) -> Option<Self> {
        match name {
            b"WinAnsiEncoding" => Some(Self::WinAnsi),
            b"MacRomanEncoding" => Some(Self::MacRoman),
            b"StandardEncoding" => Some(Self::Standard),
            b"MacExpertEncoding" => Some(Self::MacExpert),
            _ => None,
        }
    }
    pub fn pdf_name(self) -> &'static [u8] {
        match self {
            Self::WinAnsi => b"WinAnsiEncoding",
            Self::MacRoman => b"MacRomanEncoding",
            Self::Standard => b"StandardEncoding",
            Self::MacExpert => b"MacExpertEncoding",
        }
    }
    fn table(self) -> &'static [Option<&'static [u8]>; 256] {
        match self {
            Self::WinAnsi => &data::WIN_ANSI,
            Self::MacRoman => &data::MAC_ROMAN,
            Self::Standard => &data::STANDARD,
            Self::MacExpert => &data::MAC_EXPERT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodingLimits {
    pub max_items: usize,
    pub max_name_bytes: usize,
    /// All incoming name bytes, including overwritten assignments. Existing
    /// base slots have already been validated and do not consume this budget.
    pub max_processed_name_bytes: usize,
}
impl Default for EncodingLimits {
    fn default() -> Self {
        Self {
            max_items: 4096,
            max_name_bytes: 127,
            max_processed_name_bytes: 256 * 1024,
        }
    }
}

/// Borrowed differences; every name is validated before it can become a slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Difference<'a> {
    Code(u8),
    Name(&'a [u8]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlyphUnicode<'a> {
    /// Either an undefined base slot or an explicit `.notdef` assignment.
    UndefinedCode,
    UnknownGlyphName(&'a [u8]),
    Unicode(&'a [char]),
}

#[derive(Debug, Eq, PartialEq)]
pub struct EncodingError {
    /// The zero-based failing Differences array index; None for array shape or
    /// whole-array item budget. Allocation errors identify the byte slot.
    pub index: Option<usize>,
    pub kind: EncodingErrorKind,
}
#[derive(Debug, Eq, PartialEq)]
pub enum EncodingErrorKind {
    ExpectedArray,
    ExpectedCodeOrName,
    CodeOutOfRange,
    NameBeforeCode,
    CodeOverflow,
    EmptyName,
    LimitExceeded(&'static str),
    AllocationFailed { code: u8 },
}
impl fmt::Display for EncodingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PDF simple encoding")?;
        if let Some(index) = self.index {
            write!(f, " Differences[{index}]")?;
        }
        match &self.kind {
            EncodingErrorKind::ExpectedArray => f.write_str(": expected Differences array"),
            EncodingErrorKind::ExpectedCodeOrName => {
                f.write_str(": expected integer code or byte name")
            }
            EncodingErrorKind::CodeOutOfRange => f.write_str(": code must be in 0..=255"),
            EncodingErrorKind::NameBeforeCode => f.write_str(": name needs a preceding code"),
            EncodingErrorKind::CodeOverflow => {
                f.write_str(": name exceeds slot 255 without a code reset")
            }
            EncodingErrorKind::EmptyName => f.write_str(": empty glyph name"),
            EncodingErrorKind::LimitExceeded(resource) => write!(f, ": {resource} limit exceeded"),
            EncodingErrorKind::AllocationFailed { code } => {
                write!(f, ": allocation failed for slot {code}")
            }
        }
    }
}
impl std::error::Error for EncodingError {}

#[derive(Debug, Eq, PartialEq)]
pub struct SimpleEncoding {
    slots: [Option<Cow<'static, [u8]>>; 256],
}
impl SimpleEncoding {
    pub fn from_named(base: NamedEncoding) -> Self {
        Self {
            slots: std::array::from_fn(|i| base.table()[i].map(Cow::Borrowed)),
        }
    }
    pub fn glyph_name(&self, code: u8) -> Option<&[u8]> {
        self.slots[usize::from(code)].as_deref()
    }
    pub fn lookup_unicode(&self, code: u8) -> GlyphUnicode<'_> {
        let Some(name) = self.glyph_name(code) else {
            return GlyphUnicode::UndefinedCode;
        };
        if name == b".notdef" {
            return GlyphUnicode::UndefinedCode;
        }
        match data::UNICODE.binary_search_by(|(key, _)| key.cmp(&name)) {
            Ok(index) => GlyphUnicode::Unicode(data::UNICODE[index].1),
            Err(_) => GlyphUnicode::UnknownGlyphName(name),
        }
    }

    /// Transactional update: validates and charges every incoming item before
    /// allocating at most 256 final slots. Repeated assignments use the last
    /// name, without allocating discarded names or cloning an Object tree.
    pub fn with_differences(
        &self,
        entries: &[Difference<'_>],
        limits: EncodingLimits,
    ) -> Result<Self, EncodingError> {
        self.apply(entries.len(), limits, |i| Ok(entries[i]))
    }

    /// Direct Object array adapter. References and other object types are not
    /// resolved here. Errors identify the exact failing array index.
    pub fn with_object_differences(
        &self,
        value: &Object,
        limits: EncodingLimits,
    ) -> Result<Self, EncodingError> {
        let Object::Array(entries) = value else {
            return Err(EncodingError {
                index: None,
                kind: EncodingErrorKind::ExpectedArray,
            });
        };
        self.apply(entries.len(), limits, |i| match &entries[i] {
            Object::Integer(code) => u8::try_from(*code)
                .map(Difference::Code)
                .map_err(|_| EncodingErrorKind::CodeOutOfRange),
            Object::Name(name) => Ok(Difference::Name(name)),
            _ => Err(EncodingErrorKind::ExpectedCodeOrName),
        })
    }

    fn apply<'a>(
        &self,
        len: usize,
        limits: EncodingLimits,
        item: impl Fn(usize) -> Result<Difference<'a>, EncodingErrorKind>,
    ) -> Result<Self, EncodingError> {
        if len > limits.max_items {
            return Err(EncodingError {
                index: None,
                kind: EncodingErrorKind::LimitExceeded("Differences items"),
            });
        }
        let mut replacements: [Option<(&[u8], usize)>; 256] = [None; 256];
        let mut cursor: Option<usize> = None;
        let mut processed = 0usize;
        for index in 0..len {
            let error = |kind| EncodingError {
                index: Some(index),
                kind,
            };
            match item(index).map_err(error)? {
                Difference::Code(code) => cursor = Some(usize::from(code)),
                Difference::Name(name) => {
                    let code = cursor.ok_or_else(|| error(EncodingErrorKind::NameBeforeCode))?;
                    if code >= 256 {
                        return Err(error(EncodingErrorKind::CodeOverflow));
                    }
                    if name.is_empty() {
                        return Err(error(EncodingErrorKind::EmptyName));
                    }
                    if name.len() > limits.max_name_bytes {
                        return Err(error(EncodingErrorKind::LimitExceeded("glyph name bytes")));
                    }
                    processed = processed
                        .checked_add(name.len())
                        .filter(|&n| n <= limits.max_processed_name_bytes)
                        .ok_or_else(|| {
                            error(EncodingErrorKind::LimitExceeded("processed name bytes"))
                        })?;
                    replacements[code] = Some((name, index));
                    cursor = Some(
                        code.checked_add(1)
                            .ok_or_else(|| error(EncodingErrorKind::CodeOverflow))?,
                    );
                }
            }
        }
        let mut slots: [Option<Cow<'static, [u8]>>; 256] = [const { None }; 256];
        for code in 0..256 {
            let (name, index) = if let Some((name, index)) = replacements[code] {
                (Some(name), Some(index))
            } else {
                // Existing static names require no allocation or payload copy.
                if let Some(Cow::Borrowed(name)) = self.slots[code] {
                    slots[code] = Some(Cow::Borrowed(name));
                    continue;
                }
                (self.slots[code].as_deref(), None)
            };
            if let Some(name) = name {
                let mut copy = Vec::new();
                copy.try_reserve_exact(name.len())
                    .map_err(|_| EncodingError {
                        index,
                        kind: EncodingErrorKind::AllocationFailed { code: code as u8 },
                    })?;
                copy.extend_from_slice(name);
                slots[code] = Some(Cow::Owned(copy));
            }
        }
        Ok(Self { slots })
    }
}
