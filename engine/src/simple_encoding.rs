// SPDX-License-Identifier: AGPL-3.0-or-later
//! The four named simple-font encodings of ISO 32000-2 Annex D
//! (StandardEncoding, MacRomanEncoding, WinAnsiEncoding and
//! MacExpertEncoding), `/Differences` overlays on them (§9.6.5.1), and the
//! Unicode value of the glyph names they use.
//!
//! The tables are generated (`simple_encoding_data.rs`, by
//! `generate/generate_data.py`) from the standard's tables and the Adobe
//! Glyph List.
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
    /// The encoding a PDF name stands for: exactly `WinAnsiEncoding`,
    /// `MacRomanEncoding`, `StandardEncoding` or `MacExpertEncoding`.
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

    fn table(self) -> &'static [u16; 256] {
        match self {
            Self::WinAnsi => &data::WIN_ANSI,
            Self::MacRoman => &data::MAC_ROMAN,
            Self::Standard => &data::STANDARD,
            Self::MacExpert => &data::MAC_EXPERT,
        }
    }
}

/// The glyph name of `code` in a named encoding, or `None` for an undefined
/// slot.
pub(crate) fn base_glyph_name(encoding: NamedEncoding, code: u8) -> Option<&'static [u8]> {
    let index = encoding.table()[usize::from(code)];
    if index == data::UNDEFINED {
        return None;
    }
    data::REGISTRY.get(usize::from(index)).map(|entry| entry.0)
}

/// The Adobe Glyph List value of a glyph name of the four tables (the
/// registry), or `None` for any other name.
pub(crate) fn registry_unicode(name: &[u8]) -> Option<&'static [char]> {
    let index = data::REGISTRY
        .binary_search_by(|entry| entry.0.cmp(name))
        .ok()?;
    data::REGISTRY.get(index).map(|entry| &entry.1[..])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodingLimits {
    pub max_items: usize,
    pub max_name_bytes: usize,
    /// All incoming name bytes, overwritten assignments included. Base
    /// slots do not count.
    pub max_processed_name_bytes: usize,
}

impl Default for EncodingLimits {
    /// 4096 items, 127 name bytes (Annex C's name limit), 256 KiB.
    fn default() -> Self {
        Self {
            max_items: 4096,
            max_name_bytes: 127,
            max_processed_name_bytes: 256 << 10,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Difference<'a> {
    Code(u8),
    Name(&'a [u8]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlyphUnicode<'a> {
    /// An undefined base slot, or an explicit `.notdef`.
    UndefinedCode,
    UnknownGlyphName(&'a [u8]),
    Unicode(&'a [char]),
}

#[derive(Debug, Eq, PartialEq)]
pub struct EncodingError {
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
            EncodingErrorKind::LimitExceeded(resource) => {
                write!(f, ": {resource} limit exceeded")
            }
            EncodingErrorKind::AllocationFailed { code } => {
                write!(f, ": allocation failed for slot {code}")
            }
        }
    }
}

impl std::error::Error for EncodingError {}

/// A simple encoding: an optional glyph name for each of the 256 codes.
/// Names are opaque bytes.
#[derive(Debug, Eq, PartialEq)]
pub struct SimpleEncoding {
    /// 256 slots.
    slots: Vec<Option<Cow<'static, [u8]>>>,
}

impl SimpleEncoding {
    pub fn from_named(base: NamedEncoding) -> Self {
        let slots = (0..=255u8)
            .map(|code| base_glyph_name(base, code).map(Cow::Borrowed))
            .collect();
        Self { slots }
    }

    /// The glyph name of `code`, or `None` for an undefined slot.
    pub fn glyph_name(&self, code: u8) -> Option<&[u8]> {
        self.slots.get(usize::from(code))?.as_deref()
    }

    /// The Unicode value of `code`'s glyph name when the name is one of the
    /// four tables' names; this is deliberately not a general glyph-name
    /// resolver (`uni0041` is an unknown name here).
    pub fn lookup_unicode(&self, code: u8) -> GlyphUnicode<'_> {
        match self.glyph_name(code) {
            None | Some(b".notdef") => GlyphUnicode::UndefinedCode,
            Some(name) => match registry_unicode(name) {
                Some(chars) => GlyphUnicode::Unicode(chars),
                None => GlyphUnicode::UnknownGlyphName(name),
            },
        }
    }

    /// A new encoding: `self` with the `/Differences` entries applied.
    /// Every entry is checked and charged before anything is allocated;
    /// `self` never changes.
    pub fn with_differences(
        &self,
        entries: &[Difference<'_>],
        limits: EncodingLimits,
    ) -> Result<Self, EncodingError> {
        check_item_count(entries.len(), limits)?;
        let mut plan = Plan::new(limits);
        for (index, entry) in entries.iter().enumerate() {
            plan.apply(index, *entry)?;
        }
        self.build(
            |index| match entries.get(index) {
                Some(Difference::Name(name)) => Some(name),
                _ => None,
            },
            &plan,
        )
    }

    /// [`Self::with_differences`] on a `/Differences` array object: integers
    /// in 0..=255 are codes, names are glyph names, anything else (references
    /// included) is an error.
    pub fn with_object_differences(
        &self,
        value: &Object,
        limits: EncodingLimits,
    ) -> Result<Self, EncodingError> {
        let Object::Array(items) = value else {
            return Err(EncodingError {
                index: None,
                kind: EncodingErrorKind::ExpectedArray,
            });
        };
        check_item_count(items.len(), limits)?;
        let mut plan = Plan::new(limits);
        for (index, item) in items.iter().enumerate() {
            let entry = match item {
                Object::Integer(code) => match u8::try_from(*code) {
                    Ok(code) => Difference::Code(code),
                    Err(_) => {
                        return Err(EncodingError {
                            index: Some(index),
                            kind: EncodingErrorKind::CodeOutOfRange,
                        });
                    }
                },
                Object::Name(name) => Difference::Name(name),
                _ => {
                    return Err(EncodingError {
                        index: Some(index),
                        kind: EncodingErrorKind::ExpectedCodeOrName,
                    });
                }
            };
            plan.apply(index, entry)?;
        }
        self.build(
            |index| match items.get(index) {
                Some(Object::Name(name)) => Some(name.as_slice()),
                _ => None,
            },
            &plan,
        )
    }

    /// Allocates the new encoding: the slots `plan` assigns take the name of
    /// the entry (`name_of` its index), the others keep `self`'s.
    fn build<'n>(
        &self,
        name_of: impl Fn(usize) -> Option<&'n [u8]>,
        plan: &Plan,
    ) -> Result<Self, EncodingError> {
        let mut slots = Vec::new();
        slots.try_reserve_exact(256).map_err(|_| EncodingError {
            index: None,
            kind: EncodingErrorKind::AllocationFailed { code: 0 },
        })?;
        for code in 0..=255u8 {
            let slot = match plan.assigned[usize::from(code)] {
                Some(index) => match name_of(index) {
                    Some(name) => Some(owned(name, code, Some(index))?),
                    None => None,
                },
                None => match self.slots.get(usize::from(code)).and_then(Option::as_ref) {
                    Some(Cow::Borrowed(name)) => Some(Cow::Borrowed(*name)),
                    Some(Cow::Owned(name)) => Some(owned(name, code, None)?),
                    None => None,
                },
            };
            slots.push(slot);
        }
        Ok(Self { slots })
    }
}

fn check_item_count(items: usize, limits: EncodingLimits) -> Result<(), EncodingError> {
    if items > limits.max_items {
        return Err(EncodingError {
            index: None,
            kind: EncodingErrorKind::LimitExceeded("Differences items"),
        });
    }
    Ok(())
}

fn owned(name: &[u8], code: u8, index: Option<usize>) -> Result<Cow<'static, [u8]>, EncodingError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(name.len())
        .map_err(|_| EncodingError {
            index,
            kind: EncodingErrorKind::AllocationFailed { code },
        })?;
    bytes.extend_from_slice(name);
    Ok(Cow::Owned(bytes))
}

/// The checked and charged outcome of a run of `/Differences` entries: for
/// each slot, the index of the entry that sets it last.
struct Plan {
    limits: EncodingLimits,
    /// The next slot a name fills; 256 once slot 255 is filled.
    cursor: Option<u16>,
    processed: usize,
    assigned: [Option<usize>; 256],
}

impl Plan {
    fn new(limits: EncodingLimits) -> Self {
        Self {
            limits,
            cursor: None,
            processed: 0,
            assigned: [None; 256],
        }
    }

    fn apply(&mut self, index: usize, entry: Difference<'_>) -> Result<(), EncodingError> {
        let error = |kind| EncodingError {
            index: Some(index),
            kind,
        };
        match entry {
            Difference::Code(code) => self.cursor = Some(u16::from(code)),
            Difference::Name(name) => {
                let cursor = self
                    .cursor
                    .ok_or_else(|| error(EncodingErrorKind::NameBeforeCode))?;
                let slot =
                    u8::try_from(cursor).map_err(|_| error(EncodingErrorKind::CodeOverflow))?;
                if name.is_empty() {
                    return Err(error(EncodingErrorKind::EmptyName));
                }
                if name.len() > self.limits.max_name_bytes {
                    return Err(error(EncodingErrorKind::LimitExceeded("glyph name bytes")));
                }
                self.processed = self.processed.saturating_add(name.len());
                if self.processed > self.limits.max_processed_name_bytes {
                    return Err(error(EncodingErrorKind::LimitExceeded(
                        "processed name bytes",
                    )));
                }
                self.assigned[usize::from(slot)] = Some(index);
                self.cursor = Some(cursor + 1);
            }
        }
        Ok(())
    }
}
