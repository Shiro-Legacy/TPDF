// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict, bounded parsing of direct PDF objects and indirect references.
//!
//! Streams, indirect-object wrappers, cross-reference data and whole documents
//! are deliberately outside this API. `parse_one` returns the end of the object,
//! excluding trailing whitespace and any tokens read ahead to detect references.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::ops::Range;

use crate::lexer::{LexError, LexLimits, Lexer, Token, TokenKind};
use crate::object::{Object, ObjectId};

/// Limits are cumulative across `Parser::parse_next` calls. Dictionary keys
/// count as objects. Decoded bytes count names, strings and dictionary keys,
/// including overwritten values when `KeepLast` is selected. The lexer may
/// additionally retain at most two lookahead tokens (each bounded by
/// `lex.max_token_bytes`); these are charged when consumed.
#[derive(Clone, Copy, Debug)]
pub struct ParseLimits {
    /// Maximum nested array/dictionary depth; scalars require depth zero.
    pub max_depth: usize,
    pub max_objects: usize,
    pub max_decoded_bytes: usize,
    pub lex: LexLimits,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_objects: 100_000,
            max_decoded_bytes: 16 * 1024 * 1024,
            lex: LexLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DuplicateKeyPolicy {
    #[default]
    Reject,
    KeepLast,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedObject {
    pub object: Object,
    pub span: Range<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseLimitKind {
    Depth,
    Objects,
    DecodedBytes,
}

#[derive(Debug)]
pub enum ParseErrorKind {
    Lex(LexError),
    EmptyInput,
    UnexpectedEnd {
        expected: &'static str,
    },
    UnexpectedToken {
        expected: &'static str,
        found: &'static str,
    },
    UnknownKeyword(Vec<u8>),
    Unsupported(&'static str),
    InvalidReference {
        number: i64,
        generation: i64,
    },
    DuplicateKey(Vec<u8>),
    LimitExceeded(ParseLimitKind),
    TrailingData,
}

#[derive(Debug)]
pub struct ParseError {
    pub offset: usize,
    pub kind: ParseErrorKind,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PDF object error at byte {}: ", self.offset)?;
        match &self.kind {
            ParseErrorKind::Lex(error) => write!(f, "{error}"),
            ParseErrorKind::EmptyInput => f.write_str("expected an object"),
            ParseErrorKind::UnexpectedEnd { expected } => {
                write!(f, "unexpected end; expected {expected}")
            }
            ParseErrorKind::UnexpectedToken { expected, found } => {
                write!(f, "expected {expected}, found {found}")
            }
            ParseErrorKind::UnknownKeyword(keyword) => {
                write!(f, "unknown object keyword {keyword:?}")
            }
            ParseErrorKind::Unsupported(feature) => write!(f, "unsupported {feature}"),
            ParseErrorKind::InvalidReference { number, generation } => {
                write!(f, "invalid indirect reference {number} {generation} R")
            }
            ParseErrorKind::DuplicateKey(key) => write!(f, "duplicate dictionary key {key:?}"),
            ParseErrorKind::LimitExceeded(limit) => write!(f, "{limit:?} limit exceeded"),
            ParseErrorKind::TrailingData => f.write_str("trailing data after object"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ParseErrorKind::Lex(error) => Some(error),
            _ => None,
        }
    }
}

/// A parser is terminal after any error; construct another parser to retry.
/// Caller-supplied depth limits are capped at 256 to bound native stack usage,
/// including recursive destruction of the returned owned tree.
pub struct Parser<'a> {
    lexer: Lexer<'a>,
    buffered: VecDeque<Token>,
    pending_error: Option<LexError>,
    eof: bool,
    failed: bool,
    limits: ParseLimits,
    duplicate_keys: DuplicateKeyPolicy,
    objects: usize,
    decoded_bytes: usize,
    input_len: usize,
    last_end: usize,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a [u8], mut limits: ParseLimits) -> Self {
        limits.max_depth = limits.max_depth.min(256);
        Self {
            lexer: Lexer::new(input, limits.lex),
            buffered: VecDeque::new(),
            pending_error: None,
            eof: false,
            failed: false,
            limits,
            duplicate_keys: DuplicateKeyPolicy::Reject,
            objects: 0,
            decoded_bytes: 0,
            input_len: input.len(),
            last_end: 0,
        }
    }

    pub fn with_duplicate_key_policy(mut self, policy: DuplicateKeyPolicy) -> Self {
        self.duplicate_keys = policy;
        self
    }

    pub fn parse_next(&mut self) -> Result<Option<ParsedObject>, ParseError> {
        if self.failed {
            return Err(self.error(
                self.last_end,
                ParseErrorKind::Unsupported("reuse after a parser error"),
            ));
        }
        let result = self.parse_next_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn parse_next_inner(&mut self) -> Result<Option<ParsedObject>, ParseError> {
        self.fill(0);
        let start = match self.buffered.front() {
            Some(token) => token.span.start,
            None => {
                self.propagate_lex_error()?;
                return Ok(None);
            }
        };
        let object = self.parse_value(0)?;
        Ok(Some(ParsedObject {
            object,
            span: start..self.last_end,
        }))
    }

    pub fn parse_complete(mut self) -> Result<Object, ParseError> {
        let parsed = self
            .parse_next()?
            .ok_or_else(|| self.error(self.input_len, ParseErrorKind::EmptyInput))?;
        self.fill(0);
        if let Some(token) = self.buffered.front() {
            return Err(self.error(token.span.start, ParseErrorKind::TrailingData));
        }
        self.propagate_lex_error()?;
        Ok(parsed.object)
    }

    fn fill(&mut self, index: usize) {
        while self.buffered.len() <= index && !self.eof && self.pending_error.is_none() {
            match self.lexer.next_token() {
                Ok(Some(token)) => self.buffered.push_back(token),
                Ok(None) => self.eof = true,
                Err(error) => self.pending_error = Some(error),
            }
        }
    }

    fn propagate_lex_error(&mut self) -> Result<(), ParseError> {
        if let Some(error) = self.pending_error.take() {
            return Err(ParseError {
                offset: error.offset,
                kind: ParseErrorKind::Lex(error),
            });
        }
        Ok(())
    }

    fn next(&mut self, expected: &'static str) -> Result<Token, ParseError> {
        self.fill(0);
        if let Some(token) = self.buffered.pop_front() {
            self.last_end = token.span.end;
            return Ok(token);
        }
        self.propagate_lex_error()?;
        Err(self.error(self.input_len, ParseErrorKind::UnexpectedEnd { expected }))
    }

    fn error(&self, offset: usize, kind: ParseErrorKind) -> ParseError {
        ParseError { offset, kind }
    }

    fn charge_object(&mut self, offset: usize) -> Result<(), ParseError> {
        self.objects = self
            .objects
            .checked_add(1)
            .filter(|&n| n <= self.limits.max_objects)
            .ok_or_else(|| {
                self.error(
                    offset,
                    ParseErrorKind::LimitExceeded(ParseLimitKind::Objects),
                )
            })?;
        Ok(())
    }

    fn charge_bytes(&mut self, amount: usize, offset: usize) -> Result<(), ParseError> {
        self.decoded_bytes = self
            .decoded_bytes
            .checked_add(amount)
            .filter(|&n| n <= self.limits.max_decoded_bytes)
            .ok_or_else(|| {
                self.error(
                    offset,
                    ParseErrorKind::LimitExceeded(ParseLimitKind::DecodedBytes),
                )
            })?;
        Ok(())
    }

    fn parse_value(&mut self, depth: usize) -> Result<Object, ParseError> {
        let token = self.next("object")?;
        let offset = token.span.start;
        self.charge_object(offset)?;
        match token.kind {
            TokenKind::Integer(number) => {
                self.fill(0);
                let generation = match self.buffered.front().map(|t| &t.kind) {
                    Some(TokenKind::Integer(generation)) => Some(*generation),
                    _ => None,
                };
                if let Some(generation) = generation {
                    self.fill(1);
                    if matches!(self.buffered.get(1).map(|t| &t.kind), Some(TokenKind::Keyword(word)) if word == b"R")
                    {
                        let id = u32::try_from(number)
                            .ok()
                            .zip(u16::try_from(generation).ok())
                            .and_then(|(number, generation)| ObjectId::new(number, generation).ok())
                            .ok_or_else(|| {
                                self.error(
                                    offset,
                                    ParseErrorKind::InvalidReference { number, generation },
                                )
                            })?;
                        self.next("reference generation")?;
                        self.next("R")?;
                        return Ok(Object::Reference(id));
                    }
                }
                Ok(Object::Integer(number))
            }
            TokenKind::Real(number) => Ok(Object::Real(number)),
            TokenKind::Name(bytes) => {
                self.charge_bytes(bytes.len(), offset)?;
                Ok(Object::Name(bytes))
            }
            TokenKind::String(bytes) => {
                self.charge_bytes(bytes.len(), offset)?;
                Ok(Object::String(bytes))
            }
            TokenKind::Keyword(word) => match word.as_slice() {
                b"null" => Ok(Object::Null),
                b"true" => Ok(Object::Boolean(true)),
                b"false" => Ok(Object::Boolean(false)),
                b"stream" | b"endstream" => {
                    Err(self.error(offset, ParseErrorKind::Unsupported("stream syntax")))
                }
                b"obj" | b"endobj" => Err(self.error(
                    offset,
                    ParseErrorKind::Unsupported("indirect-object wrapper syntax"),
                )),
                b"xref" | b"trailer" | b"startxref" => {
                    Err(self.error(offset, ParseErrorKind::Unsupported("document syntax")))
                }
                _ => Err(self.error(offset, ParseErrorKind::UnknownKeyword(word))),
            },
            TokenKind::ArrayStart | TokenKind::DictStart => {
                if depth >= self.limits.max_depth {
                    return Err(
                        self.error(offset, ParseErrorKind::LimitExceeded(ParseLimitKind::Depth))
                    );
                }
                if matches!(token.kind, TokenKind::ArrayStart) {
                    self.parse_array(depth + 1)
                } else {
                    self.parse_dictionary(depth + 1)
                }
            }
            other => Err(self.error(
                offset,
                ParseErrorKind::UnexpectedToken {
                    expected: "object",
                    found: token_label(&other),
                },
            )),
        }
    }

    fn parse_array(&mut self, depth: usize) -> Result<Object, ParseError> {
        let mut values = Vec::new();
        loop {
            self.fill(0);
            if matches!(
                self.buffered.front().map(|t| &t.kind),
                Some(TokenKind::ArrayEnd)
            ) {
                self.next("]")?;
                return Ok(Object::Array(values));
            }
            if self.buffered.is_empty() {
                self.propagate_lex_error()?;
                return Err(self.error(
                    self.input_len,
                    ParseErrorKind::UnexpectedEnd {
                        expected: "array value or ]",
                    },
                ));
            }
            values.push(self.parse_value(depth)?);
        }
    }

    fn parse_dictionary(&mut self, depth: usize) -> Result<Object, ParseError> {
        let mut values = BTreeMap::new();
        loop {
            let token = self.next("dictionary key or >>")?;
            let offset = token.span.start;
            match token.kind {
                TokenKind::DictEnd => return Ok(Object::Dictionary(values)),
                TokenKind::Name(key) => {
                    self.charge_object(offset)?;
                    self.charge_bytes(key.len(), offset)?;
                    if self.duplicate_keys == DuplicateKeyPolicy::Reject
                        && values.contains_key(&key)
                    {
                        return Err(self.error(offset, ParseErrorKind::DuplicateKey(key)));
                    }
                    let value = self.parse_value(depth)?;
                    values.insert(key, value);
                }
                other => {
                    return Err(self.error(
                        offset,
                        ParseErrorKind::UnexpectedToken {
                            expected: "dictionary name key or >>",
                            found: token_label(&other),
                        },
                    ));
                }
            }
        }
    }
}

fn token_label(token: &TokenKind) -> &'static str {
    match token {
        TokenKind::Integer(_) => "integer",
        TokenKind::Real(_) => "real",
        TokenKind::Name(_) => "name",
        TokenKind::String(_) => "string",
        TokenKind::ArrayStart => "[",
        TokenKind::ArrayEnd => "]",
        TokenKind::DictStart => "<<",
        TokenKind::DictEnd => ">>",
        TokenKind::Keyword(_) => "keyword",
    }
}

pub fn parse_one(input: &[u8], limits: ParseLimits) -> Result<ParsedObject, ParseError> {
    Parser::new(input, limits).parse_next()?.ok_or(ParseError {
        offset: input.len(),
        kind: ParseErrorKind::EmptyInput,
    })
}

pub fn parse_complete(input: &[u8], limits: ParseLimits) -> Result<Object, ParseError> {
    Parser::new(input, limits).parse_complete()
}
