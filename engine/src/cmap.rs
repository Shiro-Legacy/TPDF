// SPDX-License-Identifier: AGPL-3.0-or-later
// Behavior references MuPDF 1.28.1 source/pdf/pdf-cmap-parse.c and pdf-cmap.c,
// Copyright (C) 2004-2021 Artifex Software, Inc. See NOTICE.
//! Strict standalone ToUnicode CMaps; no PostScript execution or font fallback.
//! Entries with malformed UTF-16 destinations are dropped, not repaired.
//! Source widths remain distinct. Counts, Unicode, overlaps and wrappers are
//! validated rather than repaired. Expanded entries and cumulative payload are
//! bounded. Sequential ranges increment only the last UTF-16 code unit.

use crate::lexer::{LexError, LexLimits, Lexer, Token, TokenKind};
use std::{collections::BTreeMap, error::Error, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CodeWidth {
    One,
    Two,
    Three,
    Four,
}
impl CodeWidth {
    pub fn byte_len(self) -> usize {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Three => 3,
            Self::Four => 4,
        }
    }
    pub fn from_byte_len(len: usize) -> Result<Self, InvalidCharacterCode> {
        match len {
            1 => Ok(Self::One),
            2 => Ok(Self::Two),
            3 => Ok(Self::Three),
            4 => Ok(Self::Four),
            _ => Err(InvalidCharacterCode),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidCharacterCode;
impl fmt::Display for InvalidCharacterCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("character code must fit its one through four byte width")
    }
}
impl Error for InvalidCharacterCode {}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CharacterCode {
    width: CodeWidth,
    value: u32,
}
impl CharacterCode {
    pub fn new(value: u32, width: CodeWidth) -> Result<Self, InvalidCharacterCode> {
        if value > (u32::MAX >> ((4 - width.byte_len()) * 8)) {
            return Err(InvalidCharacterCode);
        }
        Ok(Self { width, value })
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, InvalidCharacterCode> {
        let width = CodeWidth::from_byte_len(bytes.len())?;
        let value = bytes.iter().fold(0u32, |v, b| (v << 8) | u32::from(*b));
        Ok(Self { width, value })
    }
    pub fn value(self) -> u32 {
        self.value
    }
    pub fn width(self) -> CodeWidth {
        self.width
    }
    pub fn to_bytes(self) -> Vec<u8> {
        self.value.to_be_bytes()[4 - self.width.byte_len()..].to_vec()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSpace {
    low: CharacterCode,
    high: CharacterCode,
}
impl CodeSpace {
    pub fn low(&self) -> CharacterCode {
        self.low
    }
    pub fn high(&self) -> CharacterCode {
        self.high
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnicodeSequence {
    scalars: Vec<char>,
}
impl UnicodeSequence {
    pub fn scalars(&self) -> &[char] {
        &self.scalars
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToUnicodeMap {
    codespaces: Vec<CodeSpace>,
    mappings: BTreeMap<CharacterCode, UnicodeSequence>,
    name: Option<Vec<u8>>,
    writing_mode: u8,
}
impl ToUnicodeMap {
    pub fn lookup(&self, code: CharacterCode) -> Option<&UnicodeSequence> {
        self.mappings.get(&code)
    }
    pub fn codespaces(&self) -> &[CodeSpace] {
        &self.codespaces
    }
    pub fn mappings(&self) -> impl Iterator<Item = (&CharacterCode, &UnicodeSequence)> {
        self.mappings.iter()
    }
    pub fn name(&self) -> Option<&[u8]> {
        self.name.as_deref()
    }
    pub fn writing_mode(&self) -> u8 {
        self.writing_mode
    }
    pub fn len(&self) -> usize {
        self.mappings.len()
    }
    pub fn is_empty(&self) -> bool {
        self.mappings.is_empty()
    }
    pub fn decode_code_prefix(
        &self,
        bytes: &[u8],
    ) -> Result<(CharacterCode, usize), CodeDecodeError> {
        if bytes.is_empty() {
            return Err(CodeDecodeError::EmptyInput);
        }
        let mut value = 0u32;
        for (i, b) in bytes.iter().take(4).enumerate() {
            value = (value << 8) | u32::from(*b);
            for space in &self.codespaces {
                if space.low.width.byte_len() == i + 1
                    && value >= space.low.value
                    && value <= space.high.value
                {
                    return Ok((
                        CharacterCode {
                            width: space.low.width,
                            value,
                        },
                        i + 1,
                    ));
                }
            }
        }
        if bytes.len() < 4 {
            for space in &self.codespaces {
                let width = space.low.width.byte_len();
                if width > bytes.len() {
                    let shift = (width - bytes.len()) * 8;
                    if value >= space.low.value >> shift && value <= space.high.value >> shift {
                        return Err(CodeDecodeError::IncompleteCode);
                    }
                }
            }
        }
        Err(CodeDecodeError::OutsideCodeSpace)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeDecodeError {
    EmptyInput,
    IncompleteCode,
    OutsideCodeSpace,
}
impl fmt::Display for CodeDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid CMap code boundary: {self:?}")
    }
}
impl Error for CodeDecodeError {}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CMapLimits {
    pub max_input_bytes: usize,
    pub lex: LexLimits,
    pub max_codespaces: usize,
    pub max_records: usize,
    pub max_mappings: usize,
    pub max_utf16_units: usize,
    pub max_owned_bytes: usize,
    pub max_depth: usize,
}
impl Default for CMapLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 4 * 1024 * 1024,
            lex: LexLimits {
                max_token_bytes: 64 * 1024,
                max_tokens: 1_000_000,
            },
            max_codespaces: 64,
            max_records: 100_000,
            max_mappings: 262_144,
            max_utf16_units: 64,
            max_owned_bytes: 16 * 1024 * 1024,
            max_depth: 16,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CMapLimitKind {
    InputBytes,
    CodeSpaces,
    Records,
    Mappings,
    Utf16Units,
    OwnedBytes,
    Depth,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CMapErrorKind {
    Lex(LexError),
    LimitExceeded(CMapLimitKind),
    Invalid(&'static str),
    Unsupported(&'static str),
    InvalidUnicode(&'static str),
    DuplicateMapping(CharacterCode),
    OverlappingCodeSpaces,
    AmbiguousCodeSpaces,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CMapError {
    pub offset: usize,
    pub kind: CMapErrorKind,
}
impl fmt::Display for CMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CMap error at byte {}: {:?}", self.offset, self.kind)
    }
}
impl Error for CMapError {}
fn err(offset: usize, kind: CMapErrorKind) -> CMapError {
    CMapError { offset, kind }
}
fn invalid(offset: usize, message: &'static str) -> CMapError {
    err(offset, CMapErrorKind::Invalid(message))
}
fn limit(offset: usize, kind: CMapLimitKind) -> CMapError {
    err(offset, CMapErrorKind::LimitExceeded(kind))
}

pub fn parse_to_unicode(bytes: &[u8], limits: CMapLimits) -> Result<ToUnicodeMap, CMapError> {
    if bytes.len() > limits.max_input_bytes {
        return Err(limit(0, CMapLimitKind::InputBytes));
    }
    Parser {
        lexer: Lexer::new(bytes, limits.lex),
        limits,
        records: 0,
        entries: 0,
        owned: 0,
        depth: 0,
        map: ToUnicodeMap {
            codespaces: Vec::new(),
            mappings: BTreeMap::new(),
            name: None,
            writing_mode: 0,
        },
        mapping_offsets: BTreeMap::new(),
        metadata: 0,
    }
    .parse()
}
struct Parser<'a> {
    lexer: Lexer<'a>,
    limits: CMapLimits,
    records: usize,
    entries: usize,
    owned: usize,
    depth: usize,
    map: ToUnicodeMap,
    mapping_offsets: BTreeMap<CharacterCode, usize>,
    metadata: u8,
}
impl Parser<'_> {
    fn token(&mut self) -> Result<Token, CMapError> {
        self.optional()?
            .ok_or_else(|| invalid(self.lexer.position(), "unexpected end of CMap"))
    }
    fn optional(&mut self) -> Result<Option<Token>, CMapError> {
        self.lexer
            .next_token()
            .map_err(|e| err(e.offset, CMapErrorKind::Lex(e)))
    }
    fn keyword(&mut self, expected: &[u8]) -> Result<(), CMapError> {
        let t = self.token()?;
        if matches!(t.kind,TokenKind::Keyword(ref s) if s==expected) {
            Ok(())
        } else {
            Err(invalid(t.span.start, "unexpected keyword"))
        }
    }
    fn name_token(&mut self, expected: &[u8]) -> Result<(), CMapError> {
        let t = self.token()?;
        if matches!(t.kind,TokenKind::Name(ref s) if s==expected) {
            Ok(())
        } else {
            Err(invalid(t.span.start, "unexpected name"))
        }
    }
    fn integer(&mut self, expected: i64) -> Result<(), CMapError> {
        let t = self.token()?;
        if t.kind == TokenKind::Integer(expected) {
            Ok(())
        } else {
            Err(invalid(t.span.start, "unexpected integer"))
        }
    }
    fn depth(&self, extra: usize, offset: usize) -> Result<(), CMapError> {
        if self.depth.saturating_add(extra) > self.limits.max_depth {
            Err(limit(offset, CMapLimitKind::Depth))
        } else {
            Ok(())
        }
    }
    fn charge(&mut self, n: usize, offset: usize) -> Result<(), CMapError> {
        self.owned = self
            .owned
            .checked_add(n)
            .filter(|n| *n <= self.limits.max_owned_bytes)
            .ok_or_else(|| limit(offset, CMapLimitKind::OwnedBytes))?;
        Ok(())
    }
    fn string(&mut self) -> Result<(Vec<u8>, usize), CMapError> {
        let t = self.token()?;
        match t.kind {
            TokenKind::String(s) => Ok((s, t.span.start)),
            _ => Err(invalid(t.span.start, "expected byte string")),
        }
    }
    fn source(&mut self) -> Result<(CharacterCode, usize), CMapError> {
        let (s, o) = self.string()?;
        Ok((
            CharacterCode::from_bytes(&s)
                .map_err(|_| invalid(o, "source code requires one through four bytes"))?,
            o,
        ))
    }
    fn parse(mut self) -> Result<ToUnicodeMap, CMapError> {
        let first = self.token()?;
        let wrapped = match first.kind {
            TokenKind::Keyword(ref k) if k == b"begincmap" => false,
            TokenKind::Name(ref n) if n == b"CIDInit" => {
                self.depth(2, first.span.start)?;
                self.depth = 2;
                self.name_token(b"ProcSet")?;
                self.keyword(b"findresource")?;
                self.keyword(b"begin")?;
                self.integer(12)?;
                self.keyword(b"dict")?;
                self.keyword(b"begin")?;
                self.keyword(b"begincmap")?;
                true
            }
            _ => {
                return Err(invalid(
                    first.span.start,
                    "expected begincmap or supported CIDInit wrapper",
                ));
            }
        };
        loop {
            let t = self.token()?;
            let offset = t.span.start;
            match t.kind {
                TokenKind::Keyword(ref k) if k == b"endcmap" => break,
                TokenKind::Integer(n) => self.section(n, offset)?,
                TokenKind::Name(n) => self.metadata(n, offset)?,
                _ => {
                    return Err(err(
                        offset,
                        CMapErrorKind::Unsupported("unsupported CMap executable construct"),
                    ));
                }
            }
        }
        if wrapped {
            if self.map.name.is_none() {
                return Err(invalid(
                    self.lexer.position(),
                    "wrapped CMap requires CMapName",
                ));
            }
            self.keyword(b"CMapName")?;
            self.keyword(b"currentdict")?;
            self.name_token(b"CMap")?;
            self.keyword(b"defineresource")?;
            self.keyword(b"pop")?;
            self.keyword(b"end")?;
            self.keyword(b"end")?;
        }
        if let Some(t) = self.optional()? {
            return Err(invalid(t.span.start, "trailing CMap tokens"));
        }
        if self.map.codespaces.is_empty() {
            return Err(invalid(self.lexer.position(), "CMap requires a codespace"));
        }
        for (code, offset) in &self.mapping_offsets {
            if !self.map.codespaces.iter().any(|s| {
                s.low.width == code.width && s.low.value <= code.value && code.value <= s.high.value
            }) {
                return Err(invalid(
                    *offset,
                    "mapping source is outside declared codespaces",
                ));
            }
        }
        Ok(self.map)
    }
    fn metadata(&mut self, name: Vec<u8>, offset: usize) -> Result<(), CMapError> {
        let bit = match name.as_slice() {
            b"CMapName" => 1,
            b"CMapType" => 2,
            b"CIDSystemInfo" => 4,
            b"WMode" => 8,
            _ => {
                return Err(err(
                    offset,
                    CMapErrorKind::Unsupported("unsupported CMap definition or usecmap"),
                ));
            }
        };
        if self.metadata & bit != 0 {
            return Err(invalid(offset, "duplicate metadata definition"));
        }
        self.metadata |= bit;
        match bit {
            1 => {
                let t = self.token()?;
                match t.kind {
                    TokenKind::Name(n) => {
                        self.charge(n.len(), t.span.start)?;
                        self.map.name = Some(n)
                    }
                    _ => return Err(invalid(t.span.start, "CMapName must be a name")),
                }
            }
            2 => {
                let t = self.token()?;
                if t.kind != TokenKind::Integer(2) {
                    return Err(err(
                        t.span.start,
                        CMapErrorKind::Unsupported("only CMapType 2 is supported"),
                    ));
                }
            }
            8 => {
                let t = self.token()?;
                match t.kind {
                    TokenKind::Integer(n @ 0..=1) => self.map.writing_mode = n as u8,
                    _ => return Err(invalid(t.span.start, "WMode must be zero or one")),
                }
            }
            4 => {
                self.depth(1, offset)?;
                let t = self.token()?;
                if t.kind != TokenKind::DictStart {
                    return Err(invalid(t.span.start, "CIDSystemInfo must be a dictionary"));
                }
                let mut keys = 0u8;
                loop {
                    let t = self.token()?;
                    if t.kind == TokenKind::DictEnd {
                        break;
                    }
                    let key = match t.kind {
                        TokenKind::Name(n) => match n.as_slice() {
                            b"Registry" => 1,
                            b"Ordering" => 2,
                            b"Supplement" => 4,
                            _ => return Err(invalid(t.span.start, "unknown CIDSystemInfo key")),
                        },
                        _ => return Err(invalid(t.span.start, "CIDSystemInfo key must be a name")),
                    };
                    if keys & key != 0 {
                        return Err(invalid(t.span.start, "duplicate CIDSystemInfo key"));
                    }
                    keys |= key;
                    if key == 4 {
                        let v = self.token()?;
                        if !matches!(v.kind,TokenKind::Integer(n) if n>=0) {
                            return Err(invalid(
                                v.span.start,
                                "Supplement must be nonnegative integer",
                            ));
                        }
                    } else {
                        let (s, o) = self.string()?;
                        self.charge(s.len(), o)?;
                    }
                }
                if keys != 7 {
                    return Err(invalid(
                        offset,
                        "CIDSystemInfo requires Registry, Ordering and Supplement",
                    ));
                }
            }
            _ => unreachable!(),
        }
        self.keyword(b"def")
    }
    fn section(&mut self, count: i64, offset: usize) -> Result<(), CMapError> {
        if count < 0 {
            return Err(invalid(offset, "negative section count"));
        }
        if count > 100 {
            return Err(err(
                offset,
                CMapErrorKind::Unsupported("section count exceeds U0 maximum 100"),
            ));
        }
        let count = count as usize;
        self.records = self
            .records
            .checked_add(count)
            .filter(|n| *n <= self.limits.max_records)
            .ok_or_else(|| limit(offset, CMapLimitKind::Records))?;
        let t = self.token()?;
        let kind = match t.kind {
            TokenKind::Keyword(k) => k,
            _ => return Err(invalid(t.span.start, "expected section keyword")),
        };
        let end = match kind.as_slice() {
            b"begincodespacerange" => b"endcodespacerange".as_slice(),
            b"beginbfchar" => b"endbfchar",
            b"beginbfrange" => b"endbfrange",
            _ => {
                return Err(err(
                    t.span.start,
                    CMapErrorKind::Unsupported("unsupported mapping section"),
                ));
            }
        };
        for _ in 0..count {
            match kind.as_slice() {
                b"begincodespacerange" => {
                    let (low, o) = self.source()?;
                    let (high, _) = self.source()?;
                    self.add_space(low, high, o)?;
                }
                b"beginbfchar" => {
                    let (code, o) = self.source()?;
                    self.reserve_entries(1, o)?;
                    let (bytes, d) = self.string()?;
                    if let Some(seq) = self.destination(&bytes, d)? {
                        self.insert(code, seq, o)?;
                    }
                }
                _ => self.range()?,
            }
        }
        self.keyword(end)
    }
    fn add_space(
        &mut self,
        low: CharacterCode,
        high: CharacterCode,
        offset: usize,
    ) -> Result<(), CMapError> {
        if low.width != high.width || low.value > high.value {
            return Err(invalid(offset, "invalid codespace endpoints"));
        }
        if self.map.codespaces.len() >= self.limits.max_codespaces {
            return Err(limit(offset, CMapLimitKind::CodeSpaces));
        }
        for s in &self.map.codespaces {
            if low.width == s.low.width {
                if low.value <= s.high.value && s.low.value <= high.value {
                    return Err(err(offset, CMapErrorKind::OverlappingCodeSpaces));
                }
            } else {
                let (a, b, c, d, shift) = if low.width < s.low.width {
                    (
                        low.value,
                        high.value,
                        s.low.value,
                        s.high.value,
                        (s.low.width.byte_len() - low.width.byte_len()) * 8,
                    )
                } else {
                    (
                        s.low.value,
                        s.high.value,
                        low.value,
                        high.value,
                        (low.width.byte_len() - s.low.width.byte_len()) * 8,
                    )
                };
                if a <= d >> shift && c >> shift <= b {
                    return Err(err(offset, CMapErrorKind::AmbiguousCodeSpaces));
                }
            }
        }
        self.map.codespaces.push(CodeSpace { low, high });
        Ok(())
    }
    fn reserve_entries(&mut self, n: usize, offset: usize) -> Result<(), CMapError> {
        self.entries = self
            .entries
            .checked_add(n)
            .filter(|n| *n <= self.limits.max_mappings)
            .ok_or_else(|| limit(offset, CMapLimitKind::Mappings))?;
        Ok(())
    }
    fn unicode(&mut self, bytes: &[u8], offset: usize) -> Result<UnicodeSequence, CMapError> {
        if bytes.is_empty() || bytes.len() % 2 != 0 {
            return Err(err(
                offset,
                CMapErrorKind::InvalidUnicode("destination must contain nonempty UTF-16BE units"),
            ));
        }
        if bytes.len() / 2 > self.limits.max_utf16_units {
            return Err(limit(offset, CMapLimitKind::Utf16Units));
        }
        let units = || {
            bytes
                .chunks_exact(2)
                .map(|p| u16::from_be_bytes([p[0], p[1]]))
        };
        let mut count = 0usize;
        for scalar in char::decode_utf16(units()) {
            scalar.map_err(|_| err(offset, CMapErrorKind::InvalidUnicode("unpaired surrogate")))?;
            count += 1;
        }
        self.charge(
            count
                .checked_mul(4)
                .ok_or_else(|| limit(offset, CMapLimitKind::OwnedBytes))?,
            offset,
        )?;
        let scalars = char::decode_utf16(units())
            .map(|v| v.expect("validated UTF-16"))
            .collect();
        Ok(UnicodeSequence { scalars })
    }
    /// Producers emit malformed destinations (odd-length or unpaired UTF-16)
    /// in entries a page may never show. Such an entry is dropped: its codes
    /// have no Unicode, so showing one is still refused by decoding.
    fn destination(
        &mut self,
        bytes: &[u8],
        offset: usize,
    ) -> Result<Option<UnicodeSequence>, CMapError> {
        match self.unicode(bytes, offset) {
            Err(CMapError {
                kind: CMapErrorKind::InvalidUnicode(_),
                ..
            }) => Ok(None),
            result => result.map(Some),
        }
    }
    fn insert(
        &mut self,
        code: CharacterCode,
        seq: UnicodeSequence,
        offset: usize,
    ) -> Result<(), CMapError> {
        if self.map.mappings.contains_key(&code) {
            return Err(err(offset, CMapErrorKind::DuplicateMapping(code)));
        }
        self.map.mappings.insert(code, seq);
        self.mapping_offsets.insert(code, offset);
        Ok(())
    }
    fn range(&mut self) -> Result<(), CMapError> {
        let (low, offset) = self.source()?;
        let (high, _) = self.source()?;
        if low.width != high.width || low.value > high.value {
            return Err(invalid(offset, "invalid mapping range endpoints"));
        }
        if high.value > 65535 {
            return Err(err(
                offset,
                CMapErrorKind::Unsupported("U0 bfrange numeric sources are at most 65535"),
            ));
        }
        let count = (high.value - low.value) as usize + 1;
        self.reserve_entries(count, offset)?;
        let t = self.token()?;
        match t.kind {
            TokenKind::String(mut bytes) => {
                // Validate the base before inspecting its final code unit.
                let Some(first) = self.destination(&bytes, t.span.start)? else {
                    return Ok(());
                };
                let end = bytes.len();
                let last = u16::from_be_bytes([bytes[end - 2], bytes[end - 1]]);
                if u32::from(last) + (count - 1) as u32 > u32::from(u16::MAX) {
                    return Err(err(
                        t.span.start,
                        CMapErrorKind::Unsupported("sequential destination requires UTF-16 carry"),
                    ));
                }
                self.insert(low, first, offset)?;
                for delta in 1..count {
                    let next = (u32::from(last) + delta as u32) as u16;
                    bytes[end - 2..].copy_from_slice(&next.to_be_bytes());
                    let Some(seq) = self.destination(&bytes, t.span.start)? else {
                        continue;
                    };
                    self.insert(
                        CharacterCode {
                            width: low.width,
                            value: low.value + delta as u32,
                        },
                        seq,
                        offset,
                    )?;
                }
            }
            TokenKind::ArrayStart => {
                self.depth(1, t.span.start)?;
                for delta in 0..count {
                    let (bytes, o) = self.string()?;
                    let Some(seq) = self.destination(&bytes, o)? else {
                        continue;
                    };
                    self.insert(
                        CharacterCode {
                            width: low.width,
                            value: low.value + delta as u32,
                        },
                        seq,
                        offset,
                    )?;
                }
                let close = self.token()?;
                if close.kind != TokenKind::ArrayEnd {
                    return Err(invalid(
                        close.span.start,
                        "range destination array must have exact length",
                    ));
                }
            }
            _ => {
                return Err(invalid(
                    t.span.start,
                    "range destination must be string or array",
                ));
            }
        }
        Ok(())
    }
}
