// SPDX-License-Identifier: AGPL-3.0-or-later
// PDF lexical behavior references MuPDF 1.28.1 source/pdf/pdf-lex.c,
// Copyright (C) 2004-2024 Artifex Software, Inc. See NOTICE.
//! Strict, byte-oriented PDF tokenization with bounded token storage.
//!
//! This is a safe Rust redesign, not MuPDF's malformed-input recovery policy,
//! except in the lenient mode the repair uses (`Lexer::with_lenient`), which
//! reads damaged tokens as MuPDF's lexer reads them.
//! `max_token_bytes` counts the encoded span, including delimiters and escapes.
//! Trivia is scanned once without allocation. Errors are sticky: subsequent calls
//! return the same error, so callers cannot accidentally resume in a broken token.

use std::{error::Error, fmt, ops::Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexLimits {
    pub max_token_bytes: usize,
    pub max_tokens: usize,
}

impl Default for LexLimits {
    fn default() -> Self {
        Self {
            max_token_bytes: 1024 * 1024,
            max_tokens: 1_000_000,
        }
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PDF lexical error at byte {}: ", self.offset)?;
        match self.kind {
            LexErrorKind::TokenTooLong { limit } => {
                write!(f, "token exceeds {limit} encoded bytes")
            }
            LexErrorKind::TooManyTokens { limit } => write!(f, "token count exceeds {limit}"),
            LexErrorKind::UnexpectedByte(b) => write!(f, "unexpected byte 0x{b:02x}"),
            LexErrorKind::InvalidNameEscape => f.write_str("name escape requires two hex digits"),
            LexErrorKind::NullInName => f.write_str("a PDF name cannot contain a null byte"),
            LexErrorKind::UnterminatedString => f.write_str("unterminated literal string"),
            LexErrorKind::UnterminatedHexString => f.write_str("unterminated hexadecimal string"),
            LexErrorKind::InvalidHexDigit(b) => write!(f, "invalid hex digit 0x{b:02x}"),
            LexErrorKind::InvalidNumber => f.write_str("invalid PDF number"),
            LexErrorKind::NumberOutOfRange => f.write_str("number is outside the supported range"),
            LexErrorKind::AllocationFailed => f.write_str("token allocation failed"),
        }
    }
}

impl Error for LexError {}

#[derive(Debug)]
pub struct Lexer<'a> {
    input: &'a [u8],
    position: usize,
    emitted: usize,
    limits: LexLimits,
    failure: Option<LexError>,
    /// Damaged tokens read as MuPDF reads them (`with_lenient`).
    lenient: bool,
}

/// Private consuming cursor. The traversal retains the immutable source allocation
/// while suspended; pointer equality is an identity assertion, never dereferenced.
#[derive(Debug)]
pub(crate) struct LexerCheckpoint {
    source: *const u8,
    len: usize,
    position: usize,
    emitted: usize,
    limits: LexLimits,
    failure: Option<LexError>,
    lenient: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LexResumeError {
    SourceMismatch,
    InvalidCursor,
}

impl<'a> Lexer<'a> {
    pub(crate) fn into_checkpoint(self) -> LexerCheckpoint {
        LexerCheckpoint {
            source: self.input.as_ptr(),
            len: self.input.len(),
            position: self.position,
            emitted: self.emitted,
            limits: self.limits,
            failure: self.failure,
            lenient: self.lenient,
        }
    }
    pub(crate) fn resume(
        input: &'a [u8],
        checkpoint: LexerCheckpoint,
    ) -> Result<Self, LexResumeError> {
        if input.as_ptr() != checkpoint.source || input.len() != checkpoint.len {
            return Err(LexResumeError::SourceMismatch);
        }
        if checkpoint.position > input.len() {
            return Err(LexResumeError::InvalidCursor);
        }
        Ok(Self {
            input,
            position: checkpoint.position,
            emitted: checkpoint.emitted,
            limits: checkpoint.limits,
            failure: checkpoint.failure,
            lenient: checkpoint.lenient,
        })
    }
    pub fn new(input: &'a [u8], limits: LexLimits) -> Self {
        Self {
            input,
            position: 0,
            emitted: 0,
            limits,
            failure: None,
            lenient: false,
        }
    }

    /// Reads damaged tokens as MuPDF's lexer (`pdf-lex.c`) reads them,
    /// instead of failing (measured against MuPDF 1.28.1):
    /// - a '#' in a name that two hex digits do not follow stands for
    ///   itself (`/A#G0` is `A#G0`);
    /// - a hex string's non-hex character is the digit 0 (`<ABQD>` is
    ///   `<AB0D>`);
    /// - a number token (starting with a sign, a dot or a digit) holding a
    ///   character other than digits, one dot and '-' (`1e5`, `1.2.3`, `7g2`,
    ///   `-+5`) is a keyword, which the lenient parser reads as null; one
    ///   that holds only those is read up to its first other character
    ///   after the sign (`--5` is -5, `5-` is 5, `-` and `.` are 0), and an
    ///   integer past i64 wraps, as MuPDF's conversions do;
    /// - a keyword may hold any regular byte, and a stray `)`, `>`, `{` or
    ///   `}` is a one-byte keyword.
    pub fn with_lenient(mut self) -> Self {
        self.lenient = true;
        self
    }

    pub fn position(&self) -> usize {
        self.position
    }
    pub(crate) fn input(&self) -> &'a [u8] {
        self.input
    }
    /// Continue after raw bytes the caller consumed (inline-image data).
    pub(crate) fn seek(&mut self, position: usize) {
        self.position = position.min(self.input.len());
    }

    pub fn next_token(&mut self) -> Result<Option<Token>, LexError> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let result = self.next_inner();
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }

    fn next_inner(&mut self) -> Result<Option<Token>, LexError> {
        self.skip_trivia();
        let start = self.position;
        let Some(&first) = self.input.get(start) else {
            return Ok(None);
        };
        if self.emitted >= self.limits.max_tokens {
            return Err(self.error(
                start,
                LexErrorKind::TooManyTokens {
                    limit: self.limits.max_tokens,
                },
            ));
        }
        self.consume(start)?;
        let kind = match first {
            b'/' => self.name(start)?,
            b'(' => self.literal_string(start)?,
            b'<' if self.peek() == Some(b'<') => {
                self.consume(start)?;
                TokenKind::DictStart
            }
            b'<' => self.hex_string(start)?,
            b'>' if self.peek() == Some(b'>') => {
                self.consume(start)?;
                TokenKind::DictEnd
            }
            b'[' => TokenKind::ArrayStart,
            b']' => TokenKind::ArrayEnd,
            b')' | b'>' | b'{' | b'}' if self.lenient => TokenKind::Keyword(vec![first]),
            b')' | b'>' | b'{' | b'}' => {
                return Err(self.error(start, LexErrorKind::UnexpectedByte(first)));
            }
            _ => self.word(start)?,
        };
        self.emitted += 1;
        Ok(Some(Token {
            kind,
            span: start..self.position,
        }))
    }

    fn skip_trivia(&mut self) {
        loop {
            while self.peek().is_some_and(is_whitespace) {
                self.position += 1;
            }
            if self.peek() != Some(b'%') {
                break;
            }
            while self.peek().is_some_and(|b| b != b'\r' && b != b'\n') {
                self.position += 1;
            }
        }
    }

    fn name(&mut self, start: usize) -> Result<TokenKind, LexError> {
        let mut bytes = Vec::new();
        while let Some(b) = self.peek().filter(|&b| !is_boundary(b)) {
            let offset = self.position;
            self.consume(start)?;
            let escaped = || {
                let rest = self.input.get(self.position..)?;
                Some(hex_value(*rest.first()?).is_some() && hex_value(*rest.get(1)?).is_some())
            };
            let decoded = if b == b'#' && self.lenient && escaped() != Some(true) {
                b
            } else if b == b'#' {
                let Some(high) = self.peek().and_then(hex_value) else {
                    return Err(self.error(offset, LexErrorKind::InvalidNameEscape));
                };
                self.consume(start)?;
                let Some(low) = self.peek().and_then(hex_value) else {
                    return Err(self.error(offset, LexErrorKind::InvalidNameEscape));
                };
                self.consume(start)?;
                let value = (high << 4) | low;
                if value == 0 {
                    return Err(self.error(offset, LexErrorKind::NullInName));
                }
                value
            } else {
                b
            };
            self.push(&mut bytes, decoded, offset)?;
        }
        Ok(TokenKind::Name(bytes))
    }

    fn literal_string(&mut self, start: usize) -> Result<TokenKind, LexError> {
        let mut bytes = Vec::new();
        // Iterative balancing bounds stack use independently of string nesting.
        // Depth cannot exceed the already bounded encoded token length.
        let mut depth = 1usize;
        loop {
            let offset = self.position;
            let Some(b) = self.consume(start)? else {
                return Err(self.error(offset, LexErrorKind::UnterminatedString));
            };
            let decoded = match b {
                b'(' => {
                    depth += 1;
                    b'('
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(TokenKind::String(bytes));
                    }
                    b')'
                }
                b'\r' => {
                    if self.peek() == Some(b'\n') {
                        self.consume(start)?;
                    }
                    b'\n'
                }
                b'\\' => {
                    let Some(escaped) = self.consume(start)? else {
                        return Err(self.error(self.position, LexErrorKind::UnterminatedString));
                    };
                    match escaped {
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'b' => 8,
                        b'f' => 12,
                        b'\n' => continue,
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.consume(start)?;
                            }
                            continue;
                        }
                        b'0'..=b'7' => {
                            let mut value = u16::from(escaped - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(digit @ b'0'..=b'7') => {
                                        self.consume(start)?;
                                        value = value * 8 + u16::from(digit - b'0');
                                    }
                                    _ => break,
                                }
                            }
                            // PDF octal escapes retain the low eight bits.
                            (value & 255) as u8
                        }
                        other => other,
                    }
                }
                other => other,
            };
            self.push(&mut bytes, decoded, offset)?;
        }
    }

    fn hex_string(&mut self, start: usize) -> Result<TokenKind, LexError> {
        let mut bytes = Vec::new();
        let mut high = None;
        loop {
            let offset = self.position;
            let Some(b) = self.consume(start)? else {
                return Err(self.error(offset, LexErrorKind::UnterminatedHexString));
            };
            if b == b'>' {
                if let Some(nibble) = high {
                    self.push(&mut bytes, nibble << 4, offset)?;
                }
                return Ok(TokenKind::String(bytes));
            }
            if is_whitespace(b) {
                continue;
            }
            let nibble = match hex_value(b) {
                Some(nibble) => nibble,
                None if self.lenient => 0,
                None => return Err(self.error(offset, LexErrorKind::InvalidHexDigit(b))),
            };
            if let Some(previous) = high.take() {
                self.push(&mut bytes, (previous << 4) | nibble, offset)?;
            } else {
                high = Some(nibble);
            }
        }
    }

    fn word(&mut self, start: usize) -> Result<TokenKind, LexError> {
        while self.peek().is_some_and(|b| !is_boundary(b)) {
            self.consume(start)?;
        }
        let bytes = &self.input[start..self.position];
        if matches!(bytes[0], b'+' | b'-' | b'.' | b'0'..=b'9') {
            return self.number(bytes, start);
        }
        if let Some(index) =
            (bytes.iter().position(|&b| !(33..=126).contains(&b))).filter(|_| !self.lenient)
        {
            return Err(self.error(start + index, LexErrorKind::UnexpectedByte(bytes[index])));
        }
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(bytes.len())
            .map_err(|_| self.error(start, LexErrorKind::AllocationFailed))?;
        owned.extend_from_slice(bytes);
        Ok(TokenKind::Keyword(owned))
    }

    fn number(&self, bytes: &[u8], start: usize) -> Result<TokenKind, LexError> {
        match self.strict_number(bytes, start) {
            Err(_) if self.lenient => {
                lenient_number(bytes).map_err(|_| self.error(start, LexErrorKind::AllocationFailed))
            }
            result => result,
        }
    }

    fn strict_number(&self, bytes: &[u8], start: usize) -> Result<TokenKind, LexError> {
        let mut index = usize::from(matches!(bytes[0], b'+' | b'-'));
        let mut dots = 0;
        let mut has_digit = false;
        while index < bytes.len() {
            match bytes[index] {
                b'0'..=b'9' => has_digit = true,
                b'.' if dots == 0 => dots += 1,
                _ => return Err(self.error(start + index, LexErrorKind::InvalidNumber)),
            }
            index += 1;
        }
        if !has_digit {
            return Err(self.error(start, LexErrorKind::InvalidNumber));
        }
        let value = std::str::from_utf8(bytes)
            .map_err(|_| self.error(start, LexErrorKind::InvalidNumber))?;
        if dots == 0 {
            // An integer past i64 is read as the real it names (as hayro
            // reads it), never wrapped: a save writes the value read.
            match value.parse::<i64>() {
                Ok(integer) => Ok(TokenKind::Integer(integer)),
                Err(_) => match value.parse::<f64>() {
                    Ok(number) if number.is_finite() => Ok(TokenKind::Real(number)),
                    _ => Err(self.error(start, LexErrorKind::NumberOutOfRange)),
                },
            }
        } else {
            let number = value
                .parse::<f64>()
                .map_err(|_| self.error(start, LexErrorKind::NumberOutOfRange))?;
            if !number.is_finite() {
                return Err(self.error(start, LexErrorKind::NumberOutOfRange));
            }
            Ok(TokenKind::Real(number))
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }

    fn consume(&mut self, start: usize) -> Result<Option<u8>, LexError> {
        let Some(byte) = self.peek() else {
            return Ok(None);
        };
        if self.position - start >= self.limits.max_token_bytes {
            return Err(self.error(
                self.position,
                LexErrorKind::TokenTooLong {
                    limit: self.limits.max_token_bytes,
                },
            ));
        }
        self.position += 1;
        Ok(Some(byte))
    }

    fn push(&self, bytes: &mut Vec<u8>, byte: u8, offset: usize) -> Result<(), LexError> {
        bytes
            .try_reserve(1)
            .map_err(|_| self.error(offset, LexErrorKind::AllocationFailed))?;
        bytes.push(byte);
        Ok(())
    }

    fn error(&self, offset: usize, kind: LexErrorKind) -> LexError {
        LexError { offset, kind }
    }
}

/// MuPDF's reading of a number token the strict reader refuses
/// (`lex_number`, then `fast_atoi` or `acrobat_compatible_atof`). Extra '-'
/// after a leading '-' are skipped; a second '.' or any character other than
/// a digit, '.' or '-' makes the token a keyword. Otherwise the value is
/// read after its signs up to the first character that is not a digit (a
/// real: and its fraction). Unlike MuPDF, nothing wraps (MuPDF wraps an
/// integer past i64 and a real's integer part past u32, so a save wrote
/// another value): an integer past i64 is the real its digits name, and a
/// real's integer part is read whole; past f64 either is f64's largest.
/// The error is an allocation failure.
fn lenient_number(bytes: &[u8]) -> Result<TokenKind, std::collections::TryReserveError> {
    let first = bytes[0];
    let mut rest = &bytes[1..];
    if first == b'-' {
        while let [b'-', tail @ ..] = rest {
            rest = tail;
        }
    }
    let mut real = first == b'.';
    let mut bad = false;
    for &b in rest {
        match b {
            b'.' => {
                bad |= real;
                real = true;
            }
            b'0'..=b'9' | b'-' => {}
            _ => bad = true,
        }
    }
    if bad {
        let mut owned = Vec::new();
        owned.try_reserve_exact(bytes.len())?;
        owned.extend_from_slice(bytes);
        return Ok(TokenKind::Keyword(owned));
    }
    // The scratch text MuPDF converts: the first byte, then the rest.
    let text = std::iter::once(first).chain(rest.iter().copied());
    let mut text = text.peekable();
    let mut negative = false;
    while text.next_if_eq(&b'-').is_some() {
        negative = true;
    }
    while text.next_if_eq(&b'+').is_some() {}
    let mut digits = String::new();
    while let Some(digit) = text.next_if(u8::is_ascii_digit) {
        // Leading zeros name nothing; past f64's range the value is its
        // largest anyway, so the text kept stays bounded.
        if (digits.is_empty() && digit == b'0') || digits.len() > 400 {
            continue;
        }
        digits.push(char::from(digit));
    }
    let whole = || {
        let value = if digits.is_empty() {
            0.0
        } else {
            digits.parse::<f64>().unwrap_or(f64::MAX)
        };
        value.min(f64::MAX)
    };
    if !real {
        let magnitude = if digits.is_empty() {
            Some(0)
        } else {
            digits.parse::<i64>().ok()
        };
        return Ok(match magnitude {
            Some(value) if negative => TokenKind::Integer(-value),
            Some(value) => TokenKind::Integer(value),
            // -9223372036854775808 is the one integer whose magnitude is not.
            None if negative && digits == "9223372036854775808" => TokenKind::Integer(i64::MIN),
            None if negative => TokenKind::Real(-whole()),
            None => TokenKind::Real(whole()),
        });
    }
    let mut value = whole();
    if text.next_if_eq(&b'.').is_some() {
        let (mut fraction, mut scale) = (0.0_f64, 1.0_f64);
        while let Some(digit) = text.next_if(u8::is_ascii_digit) {
            // Digits past f64's precision change nothing; stopping keeps the
            // fraction finite however long the token is.
            if scale < 1e18 {
                fraction = fraction * 10.0 + f64::from(digit - b'0');
                scale *= 10.0;
            }
        }
        value += fraction / scale;
    }
    // MuPDF writes a negative zero as 0.
    Ok(TokenKind::Real(if negative && value != 0.0 {
        -value
    } else {
        value
    }))
}

fn is_whitespace(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}
fn is_boundary(b: u8) -> bool {
    is_whitespace(b)
        || matches!(
            b,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
}
fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
