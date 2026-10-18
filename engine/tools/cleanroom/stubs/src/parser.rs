use std::fmt;
use std::marker::PhantomData;
use std::ops::Range;

use crate::lexer::{LexError, LexLimits};
use crate::object::Object;

#[derive(Clone, Copy, Debug)]
pub struct ParseLimits {
    pub max_depth: usize,
    pub max_objects: usize,
    pub max_decoded_bytes: usize,
    pub lex: LexLimits,
}

impl Default for ParseLimits {
    fn default() -> Self {
        todo!("clean-room: spec 01")
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
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("clean-room: spec 01")
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        todo!("clean-room: spec 01")
    }
}

pub struct Parser<'a> {
    _input: PhantomData<&'a [u8]>,
}

impl<'a> Parser<'a> {
    pub fn new(_input: &'a [u8], _limits: ParseLimits) -> Self {
        todo!("clean-room: spec 01")
    }

    pub fn with_duplicate_key_policy(self, _policy: DuplicateKeyPolicy) -> Self {
        todo!("clean-room: spec 01")
    }

    pub fn with_lenient(self) -> Self {
        todo!("clean-room: spec 01")
    }

    pub fn parse_next(&mut self) -> Result<Option<ParsedObject>, ParseError> {
        todo!("clean-room: spec 01")
    }

    pub fn parse_complete(self) -> Result<Object, ParseError> {
        todo!("clean-room: spec 01")
    }
}

pub fn parse_one(_input: &[u8], _limits: ParseLimits) -> Result<ParsedObject, ParseError> {
    todo!("clean-room: spec 01")
}

pub fn parse_complete(_input: &[u8], _limits: ParseLimits) -> Result<Object, ParseError> {
    todo!("clean-room: spec 01")
}
