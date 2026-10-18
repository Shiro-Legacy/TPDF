// SPDX-License-Identifier: MIT
//! Read-only PDF content syntax, without graphics execution.
//!
//! Content operands cannot contain indirect references. Unknown ordinary
//! operator words are preserved for a later typed validation/execution layer.
//! An inline image `BI ... ID data EI` becomes one `BI` operation with a
//! dictionary and a string operand; its data never reaches the lexer.
//! File-structure keywords and stray `ID`/`EI` are forbidden.
//! Limits are cumulative across calls; errors are sticky. Depth is capped at
//! 256 even if the caller requests more, bounding recursive parser/drop stacks.

use crate::lexer::{LexError, LexLimits, Lexer, Token, TokenKind};
use std::{collections::BTreeMap, error::Error, fmt, ops::Range};

#[derive(Debug, Clone, PartialEq)]
pub enum ContentOperand {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f64),
    Name(Vec<u8>),
    String(Vec<u8>),
    Array(Vec<ContentOperand>),
    Dictionary(BTreeMap<Vec<u8>, ContentOperand>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    pub operator: Vec<u8>,
    pub operands: Vec<ContentOperand>,
    /// From the first operand (or operator) to the end of the operator token;
    /// excludes leading/trailing trivia and includes trivia between tokens.
    pub span: Range<usize>,
}

#[derive(Debug, Clone, Copy)]
pub struct ContentLimits {
    pub max_input_bytes: usize,
    pub max_depth: usize,
    /// Top-level operands in one operation; nested items also charge max_nodes.
    pub max_operands: usize,
    pub max_operations: usize,
    pub max_operator_bytes: usize,
    /// Operand values, containers and dictionary keys, including prior results.
    pub max_nodes: usize,
    /// Decoded name/string/key/operator payload bytes, including prior results.
    /// Container storage is separately bounded by max_nodes/max_operands.
    pub max_owned_bytes: usize,
    /// Cumulative token count bounds token work; token bytes bound temporary
    /// lexer storage before the parser can charge a newly returned token.
    pub lex: LexLimits,
}
impl Default for ContentLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024 * 1024,
            max_depth: 64,
            max_operands: 256,
            max_operations: 100_000,
            max_operator_bytes: 64,
            max_nodes: 1_000_000,
            max_owned_bytes: 16 * 1024 * 1024,
            lex: LexLimits {
                max_token_bytes: 1024 * 1024,
                max_tokens: 1_000_000,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentLimitKind {
    InputBytes,
    Depth,
    Operands,
    Operations,
    Nodes,
    OwnedBytes,
    OperatorBytes,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentErrorKind {
    Lex(LexError),
    LimitExceeded(ContentLimitKind),
    Unsupported(&'static str),
    StructuralKeyword(Vec<u8>),
    UnexpectedEnd,
    UnexpectedToken(&'static str),
    UnexpectedKeyword(Vec<u8>),
    DuplicateKey(Vec<u8>),
    DanglingOperands,
    AllocationFailed,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentError {
    pub offset: usize,
    pub kind: ContentErrorKind,
}
impl fmt::Display for ContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PDF content at byte {}: ", self.offset)?;
        match &self.kind {
            ContentErrorKind::Lex(error) => write!(f, "{error}"),
            ContentErrorKind::LimitExceeded(kind) => write!(f, "content limit exceeded: {kind:?}"),
            ContentErrorKind::Unsupported(reason) => write!(f, "unsupported {reason}"),
            ContentErrorKind::StructuralKeyword(word) => {
                write!(f, "file-structure keyword is forbidden: {word:?}")
            }
            ContentErrorKind::UnexpectedEnd => f.write_str("unexpected end inside operand"),
            ContentErrorKind::UnexpectedToken(expected) => write!(f, "expected {expected}"),
            ContentErrorKind::UnexpectedKeyword(word) => {
                write!(f, "operator keyword inside operand: {word:?}")
            }
            ContentErrorKind::DuplicateKey(key) => write!(f, "duplicate dictionary key: {key:?}"),
            ContentErrorKind::DanglingOperands => {
                f.write_str("operands at end of content have no operator")
            }
            ContentErrorKind::AllocationFailed => f.write_str("content operand allocation failed"),
        }
    }
}
impl Error for ContentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            ContentErrorKind::Lex(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContentUsage {
    pub nodes: usize,
    pub owned_bytes: usize,
    pub operations: usize,
    pub tokens: usize,
}

pub struct ContentParser<'a> {
    lexer: Lexer<'a>,
    limits: ContentLimits,
    tokens: usize,
    nodes: usize,
    owned_bytes: usize,
    operations: usize,
    failure: Option<ContentError>,
}
impl<'a> ContentParser<'a> {
    pub fn new(bytes: &'a [u8], mut limits: ContentLimits) -> Self {
        limits.max_depth = limits.max_depth.min(256);
        let failure = (bytes.len() > limits.max_input_bytes).then_some(ContentError {
            offset: 0,
            kind: ContentErrorKind::LimitExceeded(ContentLimitKind::InputBytes),
        });
        Self {
            lexer: Lexer::new(bytes, limits.lex),
            limits,
            tokens: 0,
            nodes: 0,
            owned_bytes: 0,
            operations: 0,
            failure,
        }
    }
    pub fn usage(&self) -> ContentUsage {
        ContentUsage {
            nodes: self.nodes,
            owned_bytes: self.owned_bytes,
            operations: self.operations,
            tokens: self.tokens,
        }
    }
    pub fn position(&self) -> usize {
        self.lexer.position()
    }
    pub fn next_operation(&mut self) -> Result<Option<Operation>, ContentError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let result = self.next_inner();
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }
    fn next_inner(&mut self) -> Result<Option<Operation>, ContentError> {
        let Some(mut token) = self.next_token()? else {
            return Ok(None);
        };
        let start = token.span.start;
        if self.operations >= self.limits.max_operations {
            return Err(self.limit(start, ContentLimitKind::Operations));
        }
        if matches!(&token.kind, TokenKind::Keyword(word) if word == b"BI") {
            return self.inline_image(start).map(Some);
        }
        let mut operands = Vec::new();
        loop {
            if let TokenKind::Keyword(word) = &token.kind {
                if word == b"BI" {
                    return Err(self.error(token.span.start, ContentErrorKind::DanglingOperands));
                }
                if !matches!(word.as_slice(), b"true" | b"false" | b"null") {
                    if word.len() > self.limits.max_operator_bytes {
                        return Err(self.limit(token.span.start, ContentLimitKind::OperatorBytes));
                    }
                    self.charge_bytes(word.len(), token.span.start)?;
                    let end = token.span.end;
                    let TokenKind::Keyword(operator) = token.kind else {
                        unreachable!()
                    };
                    self.operations += 1;
                    return Ok(Some(Operation {
                        operator,
                        operands,
                        span: start..end,
                    }));
                }
            }
            if operands.len() >= self.limits.max_operands {
                return Err(self.limit(token.span.start, ContentLimitKind::Operands));
            }
            let offset = token.span.start;
            let operand = self.operand(token, 0)?;
            operands
                .try_reserve(1)
                .map_err(|_| self.error(offset, ContentErrorKind::AllocationFailed))?;
            operands.push(operand);
            token = self
                .next_token()?
                .ok_or_else(|| self.error(self.position(), ContentErrorKind::DanglingOperands))?;
        }
    }
    fn next_token(&mut self) -> Result<Option<Token>, ContentError> {
        let token = self
            .lexer
            .next_token()
            .map_err(|error| self.error(error.offset, ContentErrorKind::Lex(error)))?;
        if token.is_some() {
            self.tokens += 1;
        }
        if let Some(Token {
            kind: TokenKind::Keyword(word),
            span,
        }) = &token
        {
            if matches!(
                word.as_slice(),
                b"R" | b"obj"
                    | b"endobj"
                    | b"stream"
                    | b"endstream"
                    | b"xref"
                    | b"trailer"
                    | b"startxref"
                    | b"newobj"
                    | b"ID"
                    | b"EI"
            ) {
                return Err(self.error(
                    span.start,
                    ContentErrorKind::StructuralKeyword(word.clone()),
                ));
            }
        }
        Ok(token)
    }
    /// Parse key/value pairs up to `ID`, then the raw data and `EI`. The data
    /// length comes from /L (or /Length), else from the unfiltered sample
    /// size, else from the first `EI` delimited by whitespace.
    fn inline_image(&mut self, start: usize) -> Result<Operation, ContentError> {
        let mut entries = BTreeMap::new();
        let id_end = loop {
            let token = self
                .lexer
                .next_token()
                .map_err(|e| self.error(e.offset, ContentErrorKind::Lex(e)))?;
            let token = token
                .ok_or_else(|| self.error(self.position(), ContentErrorKind::UnexpectedEnd))?;
            self.tokens += 1;
            match token.kind {
                TokenKind::Keyword(word) if word == b"ID" => break token.span.end,
                TokenKind::Name(key) => {
                    if entries.len() >= self.limits.max_operands {
                        return Err(self.limit(token.span.start, ContentLimitKind::Operands));
                    }
                    self.charge_node(token.span.start)?;
                    self.charge_bytes(key.len(), token.span.start)?;
                    let value = self.required_token()?;
                    let value = self.operand(value, 1)?;
                    if entries.insert(key.clone(), value).is_some() {
                        return Err(
                            self.error(token.span.start, ContentErrorKind::DuplicateKey(key))
                        );
                    }
                }
                _ => {
                    return Err(self.error(
                        token.span.start,
                        ContentErrorKind::UnexpectedToken("inline image key or ID"),
                    ));
                }
            }
        };
        let input = self.lexer.input();
        let white = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0');
        let ends_ei = |at: usize| {
            input.get(at..at + 2) == Some(b"EI")
                && input
                    .get(at + 2)
                    .is_none_or(|&b| white(b) || b"/[]<>()%".contains(&b))
        };
        // One whitespace byte separates ID from the data.
        let data_start = (id_end + 1).min(input.len());
        let int = |keys: &[&[u8]]| {
            keys.iter().find_map(|k| match entries.get(*k) {
                Some(ContentOperand::Integer(v)) => usize::try_from(*v).ok(),
                _ => None,
            })
        };
        let filtered =
            entries.contains_key(b"F".as_slice()) || entries.contains_key(b"Filter".as_slice());
        let declared = int(&[b"L", b"Length"]).or_else(|| {
            if filtered {
                return None;
            }
            let mask = matches!(entries.get(b"IM".as_slice()).or(entries.get(b"ImageMask".as_slice())), Some(ContentOperand::Boolean(true)));
            let colors = match entries.get(b"CS".as_slice()).or(entries.get(b"ColorSpace".as_slice())) {
                _ if mask => 1,
                Some(ContentOperand::Name(n)) if matches!(n.as_slice(), b"RGB" | b"DeviceRGB" | b"CalRGB") => 3,
                Some(ContentOperand::Name(n)) if matches!(n.as_slice(), b"CMYK" | b"DeviceCMYK") => 4,
                Some(ContentOperand::Name(n)) if matches!(n.as_slice(), b"G" | b"DeviceGray" | b"I" | b"Indexed") => 1,
                Some(ContentOperand::Array(items)) if matches!(items.first(), Some(ContentOperand::Name(n)) if n == b"I" || n == b"Indexed") => 1,
                _ => return None,
            };
            let bpc = if mask { 1 } else { int(&[b"BPC", b"BitsPerComponent"])? };
            let width = int(&[b"W", b"Width"])?;
            let height = int(&[b"H", b"Height"])?;
            width.checked_mul(colors)?.checked_mul(bpc)?.checked_add(7).map(|bits| bits / 8)?.checked_mul(height)
        });
        let (data_end, ei) = match declared
            .and_then(|n| data_start.checked_add(n))
            .filter(|&end| end <= input.len())
        {
            Some(end) => {
                let mut at = end;
                while at < input.len() && white(input[at]) {
                    at += 1;
                }
                if !ends_ei(at) {
                    return Err(self.error(
                        at,
                        ContentErrorKind::UnexpectedToken("EI after inline image data"),
                    ));
                }
                (end, at)
            }
            None => {
                let at = (data_start.max(1)..input.len())
                    .find(|&i| white(input[i - 1]) && ends_ei(i))
                    .ok_or_else(|| self.error(data_start, ContentErrorKind::UnexpectedEnd))?;
                // The whitespace before EI is a delimiter, not data.
                (at - 1, at)
            }
        };
        let data = input[data_start..data_end.max(data_start)].to_vec();
        self.charge_bytes(data.len(), data_start)?;
        self.lexer.seek(ei + 2);
        self.operations += 1;
        Ok(Operation {
            operator: b"BI".to_vec(),
            operands: vec![
                ContentOperand::Dictionary(entries),
                ContentOperand::String(data),
            ],
            span: start..ei + 2,
        })
    }
    fn required_token(&mut self) -> Result<Token, ContentError> {
        self.next_token()?
            .ok_or_else(|| self.error(self.position(), ContentErrorKind::UnexpectedEnd))
    }
    fn operand(&mut self, token: Token, depth: usize) -> Result<ContentOperand, ContentError> {
        let offset = token.span.start;
        self.charge_node(offset)?;
        match token.kind {
            TokenKind::Integer(value) => Ok(ContentOperand::Integer(value)),
            TokenKind::Real(value) => Ok(ContentOperand::Real(value)),
            TokenKind::Name(bytes) => {
                self.charge_bytes(bytes.len(), offset)?;
                Ok(ContentOperand::Name(bytes))
            }
            TokenKind::String(bytes) => {
                self.charge_bytes(bytes.len(), offset)?;
                Ok(ContentOperand::String(bytes))
            }
            TokenKind::Keyword(word) => match word.as_slice() {
                b"true" => Ok(ContentOperand::Boolean(true)),
                b"false" => Ok(ContentOperand::Boolean(false)),
                b"null" => Ok(ContentOperand::Null),
                _ => Err(self.error(offset, ContentErrorKind::UnexpectedKeyword(word))),
            },
            TokenKind::ArrayStart => {
                if depth >= self.limits.max_depth {
                    return Err(self.limit(offset, ContentLimitKind::Depth));
                }
                let mut array = Vec::new();
                loop {
                    let next = self.required_token()?;
                    if matches!(next.kind, TokenKind::ArrayEnd) {
                        break;
                    }
                    let element_offset = next.span.start;
                    let element = self.operand(next, depth + 1)?;
                    array.try_reserve(1).map_err(|_| {
                        self.error(element_offset, ContentErrorKind::AllocationFailed)
                    })?;
                    array.push(element);
                }
                Ok(ContentOperand::Array(array))
            }
            TokenKind::DictStart => {
                if depth >= self.limits.max_depth {
                    return Err(self.limit(offset, ContentLimitKind::Depth));
                }
                let mut dictionary = BTreeMap::new();
                loop {
                    let next = self.required_token()?;
                    if matches!(next.kind, TokenKind::DictEnd) {
                        break;
                    }
                    let key_offset = next.span.start;
                    let TokenKind::Name(key) = next.kind else {
                        return Err(self.error(
                            key_offset,
                            ContentErrorKind::UnexpectedToken("dictionary name key"),
                        ));
                    };
                    self.charge_node(key_offset)?;
                    self.charge_bytes(key.len(), key_offset)?;
                    if dictionary.contains_key(&key) {
                        return Err(self.error(key_offset, ContentErrorKind::DuplicateKey(key)));
                    }
                    let token = self.required_token()?;
                    let value = self.operand(token, depth + 1)?;
                    dictionary.insert(key, value);
                }
                Ok(ContentOperand::Dictionary(dictionary))
            }
            TokenKind::ArrayEnd | TokenKind::DictEnd => {
                Err(self.error(offset, ContentErrorKind::UnexpectedToken("operand value")))
            }
        }
    }
    fn charge_node(&mut self, offset: usize) -> Result<(), ContentError> {
        if self.nodes >= self.limits.max_nodes {
            return Err(self.limit(offset, ContentLimitKind::Nodes));
        }
        self.nodes += 1;
        Ok(())
    }
    fn charge_bytes(&mut self, bytes: usize, offset: usize) -> Result<(), ContentError> {
        let next = self
            .owned_bytes
            .checked_add(bytes)
            .filter(|&n| n <= self.limits.max_owned_bytes)
            .ok_or_else(|| self.limit(offset, ContentLimitKind::OwnedBytes))?;
        self.owned_bytes = next;
        Ok(())
    }
    fn limit(&self, offset: usize, kind: ContentLimitKind) -> ContentError {
        self.error(offset, ContentErrorKind::LimitExceeded(kind))
    }
    fn error(&self, offset: usize, kind: ContentErrorKind) -> ContentError {
        ContentError { offset, kind }
    }
}
