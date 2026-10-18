// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exact glyph-name advances for two pinned bundled Base14 substitutes.
//! The private OFL data preserves only the independently verified AFM/CFF
//! intersection. No font program, Unicode-to-glyph conversion or .notdef fallback.
use crate::simple_encoding::SimpleEncoding;
#[path = "builtin_metrics_data.rs"]
mod data;
#[path = "substitute_metrics_data.rs"]
mod substitutes;
/// A glyph's horizontal advance in units of 1/1000 text space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontAdvance(f64);
impl FontAdvance {
    fn units(value: u16) -> Self {
        Self(f64::from(value))
    }
    pub fn value(self) -> f64 {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinFace {
    Helvetica,
    TimesRoman,
    HelveticaBold,
    HelveticaBoldOblique,
}
impl BuiltinFace {
    /// Exact allowlist, used only by the opt-in display-substitution policy.
    /// No subset-prefix stripping, style guessing or platform font lookup.
    pub fn from_substitute_name(name: &[u8]) -> Option<Self> {
        match name {
            b"Arial" | b"ArialMT" => Some(Self::Helvetica),
            b"Arial-Bold" | b"Arial-BoldMT" | b"Helvetica-Bold" => Some(Self::HelveticaBold),
            b"Arial-BoldItalic" | b"Arial-BoldItalicMT" | b"Helvetica-BoldOblique" => {
                Some(Self::HelveticaBoldOblique)
            }
            _ => Self::from_pdf_name(name),
        }
    }
    pub fn from_pdf_name(name: &[u8]) -> Option<Self> {
        match name {
            b"Helvetica" => Some(Self::Helvetica),
            b"Times-Roman" => Some(Self::TimesRoman),
            _ => None,
        }
    }
    pub fn pdf_name(self) -> &'static [u8] {
        match self {
            Self::Helvetica => b"Helvetica",
            Self::TimesRoman => b"Times-Roman",
            Self::HelveticaBold => b"Helvetica-Bold",
            Self::HelveticaBoldOblique => b"Helvetica-BoldOblique",
        }
    }
    fn table(self) -> &'static [(&'static [u8], u16)] {
        match self {
            Self::Helvetica => &data::HELVETICA,
            Self::TimesRoman => &data::TIMES_ROMAN,
            Self::HelveticaBold => &substitutes::HELVETICA_BOLD,
            Self::HelveticaBoldOblique => &substitutes::HELVETICA_BOLD_OBLIQUE,
        }
    }
    /// Low-level exact glyph-name metric, including the explicit .notdef glyph.
    /// Byte-code lookup below deliberately reports .notdef as UndefinedGlyph.
    pub fn lookup_glyph(self, name: &[u8]) -> Option<FontAdvance> {
        let table = self.table();
        table
            .binary_search_by(|(key, _)| key.cmp(&name))
            .ok()
            .map(|i| FontAdvance::units(table[i].1))
    }
    pub fn glyphs(self) -> impl Iterator<Item = (&'static [u8], FontAdvance)> {
        (self.table().iter()).map(|(name, value)| (*name, FontAdvance::units(*value)))
    }
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BuiltinMetricLookup<'a> {
    UndefinedGlyph,
    MissingBuiltinMetric(&'a [u8]),
    Advance(FontAdvance),
}
#[derive(Debug)]
pub struct BuiltinMetrics {
    face: BuiltinFace,
    encoding: Box<SimpleEncoding>,
}
impl BuiltinMetrics {
    pub fn from_encoding(face: BuiltinFace, encoding: SimpleEncoding) -> Self {
        Self {
            face,
            encoding: Box::new(encoding),
        }
    }
    pub fn face(&self) -> BuiltinFace {
        self.face
    }
    pub fn encoding(&self) -> &SimpleEncoding {
        &self.encoding
    }
    pub fn glyph_name(&self, code: u8) -> Option<&[u8]> {
        self.encoding.glyph_name(code)
    }
    pub fn lookup(&self, code: u8) -> BuiltinMetricLookup<'_> {
        match self.glyph_name(code) {
            None | Some(b".notdef") => BuiltinMetricLookup::UndefinedGlyph,
            Some(name) => match self.face.lookup_glyph(name) {
                Some(value) => BuiltinMetricLookup::Advance(value),
                None => BuiltinMetricLookup::MissingBuiltinMetric(name),
            },
        }
    }
}
