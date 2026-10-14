// SPDX-License-Identifier: AGPL-3.0-or-later
//! Owned, byte-preserving PDF object values. This module does not model streams.

use std::collections::BTreeMap;
use std::fmt;

/// A valid indirect-reference target. Object zero is the free-list sentinel and
/// is rejected by this strict object parser. Generation zero is valid.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectId {
    number: u32,
    generation: u16,
}

impl ObjectId {
    pub fn new(number: u32, generation: u16) -> Result<Self, InvalidObjectId> {
        if number == 0 {
            return Err(InvalidObjectId);
        }
        Ok(Self { number, generation })
    }

    pub const fn number(self) -> u32 {
        self.number
    }

    pub const fn generation(self) -> u16 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidObjectId;

impl fmt::Display for InvalidObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("indirect object number must be greater than zero")
    }
}

impl std::error::Error for InvalidObjectId {}

/// PDF strings and names are bytes, not implicitly decoded Unicode. Dictionary
/// ordering is deterministic; duplicate keys are handled by the parser's policy.
#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f64),
    Name(Vec<u8>),
    String(Vec<u8>),
    Array(Vec<Object>),
    Dictionary(BTreeMap<Vec<u8>, Object>),
    Reference(ObjectId),
}

pub type Dictionary = BTreeMap<Vec<u8>, Object>;

impl Object {
    pub fn name(name: impl Into<Vec<u8>>) -> Self {
        Self::Name(name.into())
    }
    /// A byte string, stored as given.
    pub fn string(bytes: impl Into<Vec<u8>>) -> Self {
        Self::String(bytes.into())
    }
    /// A text string encoded by `encode_text_string`.
    pub fn text(text: &str) -> Self {
        Self::String(encode_text_string(text))
    }
    pub fn reference(id: ObjectId) -> Self {
        Self::Reference(id)
    }
    pub fn integer(value: i64) -> Self {
        Self::Integer(value)
    }
    /// `None` for NaN and infinities, which PDF cannot represent.
    pub fn real(value: f64) -> Option<Self> {
        value.is_finite().then_some(Self::Real(value))
    }
    pub fn dictionary<K: Into<Vec<u8>>>(entries: impl IntoIterator<Item = (K, Object)>) -> Self {
        Self::Dictionary(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
    pub fn array(items: impl IntoIterator<Item = Object>) -> Self {
        Self::Array(items.into_iter().collect())
    }

    pub fn as_dict(&self) -> Option<&Dictionary> {
        match self {
            Self::Dictionary(d) => Some(d),
            _ => None,
        }
    }
    pub fn as_dict_mut(&mut self) -> Option<&mut Dictionary> {
        match self {
            Self::Dictionary(d) => Some(d),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Object>> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Self::Name(n) => Some(n),
            _ => None,
        }
    }
    pub fn as_string(&self) -> Option<&[u8]> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(i) => Some(*i),
            _ => None,
        }
    }
    /// Integers and reals both read as numbers.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Integer(i) => Some(*i as f64),
            Self::Real(r) => Some(*r),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_reference(&self) -> Option<ObjectId> {
        match self {
            Self::Reference(id) => Some(*id),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

/// PDFDocEncoding codes whose Unicode value differs from the code itself
/// (ISO 32000-2 Annex D, Table D.2): 0x18..=0x1F and 0x80..=0xA0.
const PDF_DOC_SPECIAL: [(u8, char); 40] = [
    (0x18, '\u{02D8}'),
    (0x19, '\u{02C7}'),
    (0x1A, '\u{02C6}'),
    (0x1B, '\u{02D9}'),
    (0x1C, '\u{02DD}'),
    (0x1D, '\u{02DB}'),
    (0x1E, '\u{02DA}'),
    (0x1F, '\u{02DC}'),
    (0x80, '\u{2022}'),
    (0x81, '\u{2020}'),
    (0x82, '\u{2021}'),
    (0x83, '\u{2026}'),
    (0x84, '\u{2014}'),
    (0x85, '\u{2013}'),
    (0x86, '\u{0192}'),
    (0x87, '\u{2044}'),
    (0x88, '\u{2039}'),
    (0x89, '\u{203A}'),
    (0x8A, '\u{2212}'),
    (0x8B, '\u{2030}'),
    (0x8C, '\u{201E}'),
    (0x8D, '\u{201C}'),
    (0x8E, '\u{201D}'),
    (0x8F, '\u{2018}'),
    (0x90, '\u{2019}'),
    (0x91, '\u{201A}'),
    (0x92, '\u{2122}'),
    (0x93, '\u{FB01}'),
    (0x94, '\u{FB02}'),
    (0x95, '\u{0141}'),
    (0x96, '\u{0152}'),
    (0x97, '\u{0160}'),
    (0x98, '\u{0178}'),
    (0x99, '\u{017D}'),
    (0x9A, '\u{0131}'),
    (0x9B, '\u{0142}'),
    (0x9C, '\u{0153}'),
    (0x9D, '\u{0161}'),
    (0x9E, '\u{017E}'),
    (0xA0, '\u{20AC}'),
];

/// The Unicode value of a PDFDocEncoding byte. Codes the table leaves
/// undefined (controls, 0x7F, 0x9F, 0xAD) decode as their Latin-1 value.
fn pdf_doc_char(byte: u8) -> char {
    PDF_DOC_SPECIAL
        .iter()
        .find(|(code, _)| *code == byte)
        .map_or(char::from(byte), |(_, c)| *c)
}

/// The PDFDocEncoding byte for a character the table defines.
fn pdf_doc_byte(c: char) -> Option<u8> {
    match c {
        '\t' | '\n' | '\r' | ' '..='~' => Some(c as u8),
        '\u{A1}'..='\u{FF}' if c != '\u{AD}' => Some(c as u8),
        _ => PDF_DOC_SPECIAL
            .iter()
            .find(|(_, u)| *u == c)
            .map(|(code, _)| *code),
    }
}

/// Encode a text string: PDFDocEncoding when it defines every character,
/// otherwise UTF-16BE with a byte order mark.
pub fn encode_text_string(text: &str) -> Vec<u8> {
    if let Some(bytes) = text.chars().map(pdf_doc_byte).collect::<Option<Vec<u8>>>() {
        return bytes;
    }
    let mut bytes = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes
}

/// Decode a text string: UTF-16BE, UTF-16LE or UTF-8 when it starts with
/// that byte order mark, otherwise PDFDocEncoding. Malformed UTF-16 or UTF-8
/// becomes U+FFFD; an odd trailing UTF-16 byte is ignored.
pub fn decode_text_string(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| unit([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    bytes.iter().map(|&b| pdf_doc_char(b)).collect()
}
