// SPDX-License-Identifier: MIT
//! Direct PDF objects (ISO 32000-2 §7.3) read from tokens.
//!
//! The strict parser accepts exactly the standard's object syntax. The
//! lenient parser (`with_lenient`, used by repair) keeps as much of a damaged
//! container as it can: damaged values inside arrays and dictionaries read
//! as null instead of failing the whole object.

use std::collections::VecDeque;
use std::fmt;
use std::ops::Range;

use crate::lexer::{LexError, LexErrorKind, LexLimits, Lexer, Token, TokenKind};
use crate::object::{Dictionary, Object, ObjectId};

/// Container nesting is capped here whatever the caller asks, so the
/// recursion that reads containers stays bounded.
const MAX_DEPTH: usize = 256;

#[derive(Clone, Copy, Debug)]
pub struct ParseLimits {
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
            max_decoded_bytes: 16 << 20,
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

impl ParseErrorKind {
    fn duplicate(&self) -> Self {
        match self {
            Self::Lex(error) => Self::Lex(error.clone()),
            Self::EmptyInput => Self::EmptyInput,
            Self::UnexpectedEnd { expected } => Self::UnexpectedEnd { expected },
            Self::UnexpectedToken { expected, found } => Self::UnexpectedToken { expected, found },
            Self::UnknownKeyword(bytes) => Self::UnknownKeyword(bytes.clone()),
            Self::Unsupported(feature) => Self::Unsupported(feature),
            Self::InvalidReference { number, generation } => Self::InvalidReference {
                number: *number,
                generation: *generation,
            },
            Self::DuplicateKey(bytes) => Self::DuplicateKey(bytes.clone()),
            Self::LimitExceeded(kind) => Self::LimitExceeded(*kind),
            Self::TrailingData => Self::TrailingData,
        }
    }
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
            ParseErrorKind::UnknownKeyword(bytes) => {
                write!(f, "unknown object keyword {bytes:?}")
            }
            ParseErrorKind::Unsupported(feature) => write!(f, "unsupported {feature}"),
            ParseErrorKind::InvalidReference { number, generation } => {
                write!(f, "invalid indirect reference {number} {generation} R")
            }
            ParseErrorKind::DuplicateKey(bytes) => {
                write!(f, "duplicate dictionary key {bytes:?}")
            }
            ParseErrorKind::LimitExceeded(kind) => write!(f, "{kind:?} limit exceeded"),
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

/// The label of a token kind in error messages.
fn label(kind: &TokenKind) -> &'static str {
    match kind {
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

fn error<T>(offset: usize, kind: ParseErrorKind) -> Result<T, ParseError> {
    Err(ParseError { offset, kind })
}

fn lex_error(error: LexError) -> ParseError {
    ParseError {
        offset: error.offset,
        kind: ParseErrorKind::Lex(error),
    }
}

/// A token read ahead: lexical errors wait here until the parser reaches
/// them, so an object completed before a bad token is still returned.
type Lookahead = Result<Option<Token>, LexError>;

pub struct Parser<'a> {
    lexer: Lexer<'a>,
    limits: ParseLimits,
    policy: DuplicateKeyPolicy,
    lenient: bool,
    ahead: VecDeque<Lookahead>,
    objects: usize,
    decoded: usize,
    failure: Option<(usize, ParseErrorKind)>,
}

impl<'a> Parser<'a> {
    pub fn new(input: &'a [u8], limits: ParseLimits) -> Self {
        Self {
            lexer: Lexer::new(input, limits.lex),
            limits,
            policy: DuplicateKeyPolicy::Reject,
            lenient: false,
            ahead: VecDeque::new(),
            objects: 0,
            decoded: 0,
            failure: None,
        }
    }

    pub fn with_duplicate_key_policy(self, policy: DuplicateKeyPolicy) -> Self {
        Self { policy, ..self }
    }

    pub fn with_lenient(self) -> Self {
        Self {
            lexer: self.lexer.with_lenient(),
            lenient: true,
            ..self
        }
    }

    pub fn parse_next(&mut self) -> Result<Option<ParsedObject>, ParseError> {
        if let Some((offset, kind)) = &self.failure {
            return error(*offset, kind.duplicate());
        }
        let result = self.next_object();
        if let Err(failure) = &result {
            self.failure = Some((failure.offset, failure.kind.duplicate()));
        }
        result
    }

    pub fn parse_complete(mut self) -> Result<Object, ParseError> {
        let Some(parsed) = self.parse_next()? else {
            return error(self.lexer.input().len(), ParseErrorKind::EmptyInput);
        };
        match self.next_token()? {
            None => Ok(parsed.object),
            Some(token) => error(token.span.start, ParseErrorKind::TrailingData),
        }
    }

    fn next_object(&mut self) -> Result<Option<ParsedObject>, ParseError> {
        let Some(token) = self.next_token()? else {
            return Ok(None);
        };
        let start = token.span.start;
        let (object, end) = self.value(token, 0)?;
        Ok(Some(ParsedObject {
            object,
            span: start..end,
        }))
    }

    fn fill(&mut self, count: usize) {
        while self.ahead.len() < count {
            let next = self.lexer.next_token();
            self.ahead.push_back(next);
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, ParseError> {
        let next = match self.ahead.pop_front() {
            Some(next) => next,
            None => self.lexer.next_token(),
        };
        next.map_err(lex_error)
    }

    /// The kind of the token `ahead` places after the current one, if it
    /// read without error.
    fn peek(&mut self, ahead: usize) -> Option<&TokenKind> {
        self.fill(ahead + 1);
        match self.ahead.get(ahead) {
            Some(Ok(Some(token))) => Some(&token.kind),
            _ => None,
        }
    }

    /// Whether the token `ahead` places after the current one is the end
    /// of input (a read-ahead error is not).
    fn peek_is_end(&mut self, ahead: usize) -> bool {
        self.fill(ahead + 1);
        matches!(self.ahead.get(ahead), Some(Ok(None)))
    }

    /// Discards a read-ahead token; its error, if any, surfaces.
    fn skip(&mut self) -> Result<Option<usize>, ParseError> {
        Ok(self.next_token()?.map(|token| token.span.end))
    }

    fn count(&mut self, offset: usize) -> Result<(), ParseError> {
        self.objects += 1;
        if self.objects > self.limits.max_objects {
            return error(
                offset,
                ParseErrorKind::LimitExceeded(ParseLimitKind::Objects),
            );
        }
        Ok(())
    }

    fn charge(&mut self, bytes: usize, offset: usize) -> Result<(), ParseError> {
        self.decoded = self.decoded.saturating_add(bytes);
        if self.decoded > self.limits.max_decoded_bytes {
            return error(
                offset,
                ParseErrorKind::LimitExceeded(ParseLimitKind::DecodedBytes),
            );
        }
        Ok(())
    }

    /// Reads the value that begins with `token`, inside containers nested
    /// `depth` deep. Returns the value and the end of its last token.
    fn value(&mut self, token: Token, depth: usize) -> Result<(Object, usize), ParseError> {
        let Token { kind, span } = token;
        self.count(span.start)?;
        let object = match kind {
            TokenKind::Integer(number) => return self.integer(number, span, depth),
            TokenKind::Real(value) => Object::Real(value),
            TokenKind::Name(bytes) => {
                self.charge(bytes.len(), span.start)?;
                Object::Name(bytes)
            }
            TokenKind::String(bytes) => {
                self.charge(bytes.len(), span.start)?;
                Object::String(bytes)
            }
            TokenKind::ArrayStart => return self.array(span.start, depth),
            TokenKind::DictStart => return self.dictionary(span.start, depth),
            TokenKind::Keyword(word) => self.keyword(word, span.start, depth)?,
            TokenKind::ArrayEnd | TokenKind::DictEnd => {
                return error(
                    span.start,
                    ParseErrorKind::UnexpectedToken {
                        expected: "object",
                        found: label(&kind),
                    },
                );
            }
        };
        Ok((object, span.end))
    }

    fn keyword(&self, word: Vec<u8>, offset: usize, depth: usize) -> Result<Object, ParseError> {
        match word.as_slice() {
            b"null" => Ok(Object::Null),
            b"true" => Ok(Object::Boolean(true)),
            b"false" => Ok(Object::Boolean(false)),
            // Inside a container, the lenient parser reads any other word
            // (a damaged number, `R`, `obj`, a stray delimiter) as null.
            _ if self.lenient && depth > 0 => Ok(Object::Null),
            b"stream" | b"endstream" => error(offset, ParseErrorKind::Unsupported("stream syntax")),
            b"obj" | b"endobj" => error(
                offset,
                ParseErrorKind::Unsupported("indirect-object wrapper syntax"),
            ),
            b"xref" | b"trailer" | b"startxref" => {
                error(offset, ParseErrorKind::Unsupported("document syntax"))
            }
            _ => error(offset, ParseErrorKind::UnknownKeyword(word)),
        }
    }

    /// An integer, or the reference `number generation R` when the next two
    /// tokens complete one.
    fn integer(
        &mut self,
        number: i64,
        span: Range<usize>,
        depth: usize,
    ) -> Result<(Object, usize), ParseError> {
        let generation = match self.peek(0) {
            Some(&TokenKind::Integer(generation)) => generation,
            _ => return Ok((Object::Integer(number), span.end)),
        };
        if !matches!(self.peek(1), Some(TokenKind::Keyword(word)) if word == b"R") {
            return Ok((Object::Integer(number), span.end));
        }
        self.skip()?;
        let end = self.skip()?.unwrap_or(span.end);
        Ok((self.reference(number, generation, span.start, depth)?, end))
    }

    fn reference(
        &self,
        number: i64,
        generation: i64,
        offset: usize,
        depth: usize,
    ) -> Result<Object, ParseError> {
        let id = u32::try_from(number)
            .ok()
            .zip(u16::try_from(generation).ok())
            .and_then(|(number, generation)| ObjectId::new(number, generation).ok());
        match id {
            Some(id) => Ok(Object::Reference(id)),
            // An impossible reference reads as what it resolves to: null.
            None if self.lenient && depth > 0 => Ok(Object::Null),
            None => error(
                offset,
                ParseErrorKind::InvalidReference { number, generation },
            ),
        }
    }

    fn open(&self, offset: usize, depth: usize) -> Result<(), ParseError> {
        if depth >= self.limits.max_depth.min(MAX_DEPTH) {
            return error(offset, ParseErrorKind::LimitExceeded(ParseLimitKind::Depth));
        }
        Ok(())
    }

    fn array(&mut self, start: usize, depth: usize) -> Result<(Object, usize), ParseError> {
        self.open(start, depth)?;
        let mut items = Vec::new();
        loop {
            let Some(token) = self.next_token()? else {
                return error(
                    self.lexer.input().len(),
                    ParseErrorKind::UnexpectedEnd { expected: "]" },
                );
            };
            match token.kind {
                TokenKind::ArrayEnd => return Ok((Object::Array(items), token.span.end)),
                // Lenient: a stray `>>` inside an array is null.
                TokenKind::DictEnd if self.lenient => {
                    self.count(token.span.start)?;
                    items.push(Object::Null);
                }
                _ => {
                    let (item, _) = self.value(token, depth + 1)?;
                    if items.len() == items.capacity() && items.try_reserve(1).is_err() {
                        return Err(lex_error(LexError {
                            offset: start,
                            kind: LexErrorKind::AllocationFailed,
                        }));
                    }
                    items.push(item);
                }
            }
        }
    }

    fn dictionary(&mut self, start: usize, depth: usize) -> Result<(Object, usize), ParseError> {
        self.open(start, depth)?;
        let mut entries = Dictionary::new();
        loop {
            let Some(token) = self.next_token()? else {
                return error(
                    self.lexer.input().len(),
                    ParseErrorKind::UnexpectedEnd { expected: ">>" },
                );
            };
            let key = match token.kind {
                TokenKind::DictEnd => return Ok((Object::Dictionary(entries), token.span.end)),
                TokenKind::Name(key) => key,
                other => {
                    return error(
                        token.span.start,
                        ParseErrorKind::UnexpectedToken {
                            expected: "name",
                            found: label(&other),
                        },
                    );
                }
            };
            self.count(token.span.start)?;
            self.charge(key.len(), token.span.start)?;
            if self.policy == DuplicateKeyPolicy::Reject && entries.contains_key(key.as_slice()) {
                return error(token.span.start, ParseErrorKind::DuplicateKey(key));
            }
            let value = self.dictionary_value(depth)?;
            entries.insert(key, value);
        }
    }

    fn dictionary_value(&mut self, depth: usize) -> Result<Object, ParseError> {
        let Some(token) = self.next_token()? else {
            return error(
                self.lexer.input().len(),
                ParseErrorKind::UnexpectedEnd { expected: "object" },
            );
        };
        if !self.lenient {
            return Ok(self.value(token, depth + 1)?.0);
        }
        match token.kind {
            // A stray `]` where a value belongs is null.
            TokenKind::ArrayEnd => {
                self.count(token.span.start)?;
                Ok(Object::Null)
            }
            TokenKind::Integer(number) => self.lenient_integer_value(number, token.span, depth),
            _ => Ok(self.value(token, depth + 1)?.0),
        }
    }

    /// A dictionary value that begins with an integer, read leniently (spec
    /// 01 §4.2 item 4): the integer before a name, `>>` or the end; a
    /// reference when `G R` follows; otherwise null, consuming the token
    /// after it (and the one after that when it is an integer).
    fn lenient_integer_value(
        &mut self,
        number: i64,
        span: Range<usize>,
        depth: usize,
    ) -> Result<Object, ParseError> {
        let integer_follows = match self.peek(0) {
            // A name, `>>`, the end, or a read-ahead error that surfaces
            // next: the integer itself.
            None | Some(TokenKind::Name(_) | TokenKind::DictEnd) => {
                self.count(span.start)?;
                return Ok(Object::Integer(number));
            }
            Some(TokenKind::Integer(_)) => true,
            Some(_) => false,
        };
        if integer_follows && matches!(self.peek(1), Some(TokenKind::Keyword(word)) if word == b"R")
        {
            let token = Token {
                kind: TokenKind::Integer(number),
                span,
            };
            return Ok(self.value(token, depth + 1)?.0);
        }
        self.count(span.start)?;
        self.skip()?;
        if integer_follows && !self.peek_is_end(0) {
            self.skip()?;
        }
        Ok(Object::Null)
    }
}

pub fn parse_one(input: &[u8], limits: ParseLimits) -> Result<ParsedObject, ParseError> {
    match Parser::new(input, limits).parse_next()? {
        Some(parsed) => Ok(parsed),
        None => error(input.len(), ParseErrorKind::EmptyInput),
    }
}

pub fn parse_complete(input: &[u8], limits: ParseLimits) -> Result<Object, ParseError> {
    Parser::new(input, limits).parse_complete()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lenient parsing of arbitrary bytes terminates without panicking and
    /// with offsets inside the input.
    #[test]
    fn lenient_parsing_of_hostile_bytes_terminates() {
        const ALPHABET: &[u8] = b"[]<>(){}/%#\\0123456789Rnulltrueobj .+-eE\r\n\0\xa5";
        let limits = ParseLimits {
            max_depth: 8,
            max_objects: 64,
            max_decoded_bytes: 256,
            lex: LexLimits {
                max_token_bytes: 64,
                max_tokens: 96,
            },
        };
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        for case in 0..20_000usize {
            let input: Vec<u8> = (0..case % 200)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    ALPHABET[seed as usize % ALPHABET.len()]
                })
                .collect();
            let mut parser = Parser::new(&input, limits)
                .with_duplicate_key_policy(DuplicateKeyPolicy::KeepLast)
                .with_lenient();
            let mut end = 0;
            loop {
                match parser.parse_next() {
                    Ok(Some(parsed)) => {
                        assert!(parsed.span.start >= end && parsed.span.end <= input.len());
                        end = parsed.span.end;
                    }
                    Ok(None) => break,
                    Err(error) => {
                        assert!(error.offset <= input.len());
                        assert!(parser.parse_next().is_err());
                        break;
                    }
                }
            }
        }
    }
}
