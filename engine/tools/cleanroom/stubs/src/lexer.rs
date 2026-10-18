use std::{error::Error, fmt, marker::PhantomData, ops::Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexLimits {
    pub max_token_bytes: usize,
    pub max_tokens: usize,
}

impl Default for LexLimits {
    fn default() -> Self {
        todo!("clean-room: spec 01")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Integer(i64),
    Real(f64),
    Name(Vec<u8>),
    String(Vec<u8>),
    ArrayStart,
    ArrayEnd,
    DictStart,
    DictEnd,
    Keyword(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    pub offset: usize,
    pub kind: LexErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LexErrorKind {
    TokenTooLong { limit: usize },
    TooManyTokens { limit: usize },
    UnexpectedByte(u8),
    InvalidNameEscape,
    NullInName,
    UnterminatedString,
    UnterminatedHexString,
    InvalidHexDigit(u8),
    InvalidNumber,
    NumberOutOfRange,
    AllocationFailed,
}

impl fmt::Display for LexError {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("clean-room: spec 01")
    }
}

impl Error for LexError {}

#[derive(Debug)]
pub struct Lexer<'a> {
    _input: PhantomData<&'a [u8]>,
}

impl<'a> Lexer<'a> {
    pub fn new(_input: &'a [u8], _limits: LexLimits) -> Self {
        todo!("clean-room: spec 01")
    }

    pub fn with_lenient(self) -> Self {
        todo!("clean-room: spec 01")
    }

    pub fn position(&self) -> usize {
        todo!("clean-room: spec 01")
    }

    pub fn next_token(&mut self) -> Result<Option<Token>, LexError> {
        todo!("clean-room: spec 01")
    }

    pub(crate) fn input(&self) -> &'a [u8] {
        todo!("clean-room: spec 01")
    }

    pub(crate) fn seek(&mut self, _position: usize) {
        todo!("clean-room: spec 01")
    }
}
