use std::fmt;

use crate::object::Object;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NamedEncoding {
    WinAnsi,
    MacRoman,
    Standard,
    MacExpert,
}

impl NamedEncoding {
    pub fn from_pdf_name(_name: &[u8]) -> Option<Self> {
        todo!("clean-room: spec 05")
    }

    pub fn pdf_name(self) -> &'static [u8] {
        todo!("clean-room: spec 05")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodingLimits {
    pub max_items: usize,
    pub max_name_bytes: usize,
    pub max_processed_name_bytes: usize,
}

impl Default for EncodingLimits {
    fn default() -> Self {
        todo!("clean-room: spec 05")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Difference<'a> {
    Code(u8),
    Name(&'a [u8]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlyphUnicode<'a> {
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
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("clean-room: spec 05")
    }
}

impl std::error::Error for EncodingError {}

#[derive(Debug, Eq, PartialEq)]
pub struct SimpleEncoding {
    _slots: (),
}

impl SimpleEncoding {
    pub fn from_named(_base: NamedEncoding) -> Self {
        todo!("clean-room: spec 05")
    }

    pub fn glyph_name(&self, _code: u8) -> Option<&[u8]> {
        todo!("clean-room: spec 05")
    }

    pub fn lookup_unicode(&self, _code: u8) -> GlyphUnicode<'_> {
        todo!("clean-room: spec 05")
    }

    pub fn with_differences(
        &self,
        _entries: &[Difference<'_>],
        _limits: EncodingLimits,
    ) -> Result<Self, EncodingError> {
        todo!("clean-room: spec 05")
    }

    pub fn with_object_differences(
        &self,
        _value: &Object,
        _limits: EncodingLimits,
    ) -> Result<Self, EncodingError> {
        todo!("clean-room: spec 05")
    }
}
