// SPDX-License-Identifier: AGPL-3.0-or-later
//! PDF tokens (ISO 32000-2 §7.2 and §7.3): white space, comments, names,
//! literal and hexadecimal strings, numbers, delimiters and keywords.
//!
//! The strict lexer refuses anything outside the standard's grammar. The
//! lenient lexer (`with_lenient`) reads damaged tokens the way a repair pass
//! needs them: bad name escapes and hex digits are tolerated, stray
//! delimiters and damaged numbers become keywords or the numbers their
//! leading digits name.

use std::{error::Error, fmt, ops::Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LexLimits {
    pub max_token_bytes: usize,
    pub max_tokens: usize,
}

impl Default for LexLimits {
    fn default() -> Self {
        Self {
            max_token_bytes: 1_048_576,
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

impl fmt::Display for LexErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TokenTooLong { limit } => write!(f, "token exceeds {limit} encoded bytes"),
            Self::TooManyTokens { limit } => write!(f, "token count exceeds {limit}"),
            Self::UnexpectedByte(byte) => write!(f, "unexpected byte 0x{byte:02x}"),
            Self::InvalidNameEscape => f.write_str("name escape requires two hex digits"),
            Self::NullInName => f.write_str("a PDF name cannot contain a null byte"),
            Self::UnterminatedString => f.write_str("unterminated literal string"),
            Self::UnterminatedHexString => f.write_str("unterminated hexadecimal string"),
            Self::InvalidHexDigit(byte) => write!(f, "invalid hex digit 0x{byte:02x}"),
            Self::InvalidNumber => f.write_str("invalid PDF number"),
            Self::NumberOutOfRange => f.write_str("number is outside the supported range"),
            Self::AllocationFailed => f.write_str("token allocation failed"),
        }
    }
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PDF lexical error at byte {}: {}",
            self.offset, self.kind
        )
    }
}

impl Error for LexError {}

/// White space (ISO 32000-2 §7.2.3, Table 1). Vertical tab is not one.
fn is_white(byte: u8) -> bool {
    matches!(byte, 0x00 | 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

/// Delimiters (ISO 32000-2 §7.2.3, Table 2).
fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_regular(byte: u8) -> bool {
    !is_white(byte) && !is_delimiter(byte)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug)]
pub struct Lexer<'a> {
    input: &'a [u8],
    position: usize,
    limits: LexLimits,
    lenient: bool,
    tokens: usize,
    failure: Option<LexError>,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a [u8], limits: LexLimits) -> Self {
        Self {
            input,
            position: 0,
            limits,
            lenient: false,
            tokens: 0,
            failure: None,
        }
    }

    pub fn with_lenient(self) -> Self {
        Self {
            lenient: true,
            ..self
        }
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn next_token(&mut self) -> Result<Option<Token>, LexError> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        match self.read_token() {
            Ok(token) => Ok(token),
            Err(failure) => {
                self.failure = Some(failure.clone());
                Err(failure)
            }
        }
    }

    pub(crate) fn input(&self) -> &'a [u8] {
        self.input
    }

    /// Moves the read position (clamped to the input). A stored failure
    /// stays, and so does the position it left.
    pub(crate) fn seek(&mut self, position: usize) {
        if self.failure.is_none() {
            self.position = position.min(self.input.len());
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.input.get(self.position.checked_add(ahead)?).copied()
    }

    fn fail<T>(&self, offset: usize, kind: LexErrorKind) -> Result<T, LexError> {
        Err(LexError { offset, kind })
    }

    /// Consumes one byte of the token that began at `start`, failing at the
    /// byte that would make the token longer than the limit.
    fn bump(&mut self, start: usize) -> Result<(), LexError> {
        if self.position - start >= self.limits.max_token_bytes {
            return self.fail(
                self.position,
                LexErrorKind::TokenTooLong {
                    limit: self.limits.max_token_bytes,
                },
            );
        }
        self.position += 1;
        Ok(())
    }

    fn push(&self, out: &mut Vec<u8>, byte: u8) -> Result<(), LexError> {
        if out.len() == out.capacity() && out.try_reserve(1).is_err() {
            return self.fail(self.position, LexErrorKind::AllocationFailed);
        }
        out.push(byte);
        Ok(())
    }

    fn skip_trivia(&mut self) {
        while let Some(byte) = self.peek() {
            if is_white(byte) {
                self.position += 1;
            } else if byte == b'%' {
                while let Some(byte) = self.peek() {
                    if byte == b'\r' || byte == b'\n' {
                        break;
                    }
                    self.position += 1;
                }
            } else {
                break;
            }
        }
    }

    fn read_token(&mut self) -> Result<Option<Token>, LexError> {
        self.skip_trivia();
        let start = self.position;
        let Some(first) = self.peek() else {
            return Ok(None);
        };
        if self.tokens >= self.limits.max_tokens {
            return self.fail(
                start,
                LexErrorKind::TooManyTokens {
                    limit: self.limits.max_tokens,
                },
            );
        }
        let kind = match first {
            b'/' => self.name(start)?,
            b'(' => self.literal_string(start)?,
            b'<' => {
                self.bump(start)?;
                if self.peek() == Some(b'<') {
                    self.bump(start)?;
                    TokenKind::DictStart
                } else {
                    self.hex_string(start)?
                }
            }
            b'>' if self.peek_at(1) == Some(b'>') => {
                self.bump(start)?;
                self.bump(start)?;
                TokenKind::DictEnd
            }
            b'[' => {
                self.bump(start)?;
                TokenKind::ArrayStart
            }
            b']' => {
                self.bump(start)?;
                TokenKind::ArrayEnd
            }
            b')' | b'>' | b'{' | b'}' => self.stray(start, first)?,
            _ => self.regular_run(start)?,
        };
        self.tokens += 1;
        Ok(Some(Token {
            kind,
            span: start..self.position,
        }))
    }

    /// A delimiter that cannot start a token: an error, or a one-byte
    /// keyword in lenient mode.
    fn stray(&mut self, start: usize, byte: u8) -> Result<TokenKind, LexError> {
        if !self.lenient {
            return self.fail(start, LexErrorKind::UnexpectedByte(byte));
        }
        self.bump(start)?;
        Ok(TokenKind::Keyword(vec![byte]))
    }

    /// A name (§7.3.5): `/` then regular bytes, `#hh` escapes decoded.
    fn name(&mut self, start: usize) -> Result<TokenKind, LexError> {
        self.bump(start)?;
        let mut out = Vec::new();
        while let Some(byte) = self.peek() {
            if !is_regular(byte) {
                break;
            }
            if byte == b'#' {
                let escape = self.peek_at(1).and_then(hex_value).and_then(|high| {
                    let low = self.peek_at(2).and_then(hex_value)?;
                    Some(high << 4 | low)
                });
                match escape {
                    Some(0) if !self.lenient => {
                        return self.fail(self.position, LexErrorKind::NullInName);
                    }
                    Some(value) if value != 0 => {
                        for _ in 0..3 {
                            self.bump(start)?;
                        }
                        self.push(&mut out, value)?;
                        continue;
                    }
                    // Lenient: a `#` that starts no escape, or starts `#00`,
                    // stands for itself.
                    _ if self.lenient => {}
                    _ => return self.fail(self.position, LexErrorKind::InvalidNameEscape),
                }
            }
            self.bump(start)?;
            self.push(&mut out, byte)?;
        }
        Ok(TokenKind::Name(out))
    }

    /// A literal string (§7.3.4.2). Nesting is counted, not recursed.
    fn literal_string(&mut self, start: usize) -> Result<TokenKind, LexError> {
        self.bump(start)?;
        let mut out = Vec::new();
        let mut depth = 1usize;
        loop {
            let Some(byte) = self.peek() else {
                return self.fail(self.input.len(), LexErrorKind::UnterminatedString);
            };
            self.bump(start)?;
            match byte {
                b'\\' => {
                    let Some(escaped) = self.peek() else {
                        return self.fail(self.input.len(), LexErrorKind::UnterminatedString);
                    };
                    self.bump(start)?;
                    let value = match escaped {
                        b'n' => Some(b'\n'),
                        b'r' => Some(b'\r'),
                        b't' => Some(b'\t'),
                        b'b' => Some(0x08),
                        b'f' => Some(0x0C),
                        b'0'..=b'7' => {
                            let mut value = u32::from(escaped - b'0');
                            for _ in 0..2 {
                                match self.peek() {
                                    Some(digit @ b'0'..=b'7') => {
                                        self.bump(start)?;
                                        value = value * 8 + u32::from(digit - b'0');
                                    }
                                    _ => break,
                                }
                            }
                            // High-order overflow is ignored (§7.3.4.2).
                            Some((value & 0xFF) as u8)
                        }
                        // A backslash before an end-of-line marker continues
                        // the line and produces nothing.
                        b'\r' => {
                            if self.peek() == Some(b'\n') {
                                self.bump(start)?;
                            }
                            None
                        }
                        b'\n' => None,
                        // `\(`, `\)`, `\\`, and a backslash before any other
                        // byte, which is dropped.
                        other => Some(other),
                    };
                    if let Some(value) = value {
                        self.push(&mut out, value)?;
                    }
                }
                b'(' => {
                    depth += 1;
                    self.push(&mut out, byte)?;
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(TokenKind::String(out));
                    }
                    self.push(&mut out, byte)?;
                }
                // An unescaped end-of-line marker reads as one LF.
                b'\r' => {
                    if self.peek() == Some(b'\n') {
                        self.bump(start)?;
                    }
                    self.push(&mut out, b'\n')?;
                }
                _ => self.push(&mut out, byte)?,
            }
        }
    }

    /// A hexadecimal string (§7.3.4.3); the `<` is already consumed.
    fn hex_string(&mut self, start: usize) -> Result<TokenKind, LexError> {
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        loop {
            let Some(byte) = self.peek() else {
                return self.fail(self.input.len(), LexErrorKind::UnterminatedHexString);
            };
            if byte == b'>' {
                self.bump(start)?;
                break;
            }
            if is_white(byte) {
                self.bump(start)?;
                continue;
            }
            let digit = match hex_value(byte) {
                Some(digit) => digit,
                None if self.lenient => 0,
                None => return self.fail(self.position, LexErrorKind::InvalidHexDigit(byte)),
            };
            self.bump(start)?;
            match high.take() {
                Some(value) => self.push(&mut out, value << 4 | digit)?,
                None => high = Some(digit),
            }
        }
        // An odd final digit is followed by an implied 0.
        if let Some(value) = high {
            self.push(&mut out, value << 4)?;
        }
        Ok(TokenKind::String(out))
    }

    /// A maximal run of regular bytes: a number or a keyword.
    fn regular_run(&mut self, start: usize) -> Result<TokenKind, LexError> {
        while self.peek().is_some_and(is_regular) {
            self.bump(start)?;
        }
        let text = self.input.get(start..self.position).unwrap_or_default();
        let Some(&first) = text.first() else {
            return self.fail(start, LexErrorKind::InvalidNumber);
        };
        if matches!(first, b'+' | b'-' | b'.' | b'0'..=b'9') {
            return match strict_number(text) {
                Ok(kind) => Ok(kind),
                Err(_) if self.lenient => self.keyword_or_number(text),
                Err((at, kind)) => self.fail(start + at, kind),
            };
        }
        if !self.lenient {
            if let Some((at, &byte)) = text
                .iter()
                .enumerate()
                .find(|(_, byte)| !(0x21..=0x7E).contains(*byte))
            {
                return self.fail(start + at, LexErrorKind::UnexpectedByte(byte));
            }
        }
        self.keyword(text)
    }

    fn keyword(&self, text: &[u8]) -> Result<TokenKind, LexError> {
        let mut out = Vec::new();
        if out.try_reserve_exact(text.len()).is_err() {
            return self.fail(self.position, LexErrorKind::AllocationFailed);
        }
        out.extend_from_slice(text);
        Ok(TokenKind::Keyword(out))
    }

    /// Lenient reading of a token that starts like a number but is not a
    /// valid one (spec 01 §4.1 item 4): a keyword when, past its first byte
    /// (and the run of `-` after a leading `-`), it holds anything but
    /// digits, `.` and `-`, or two points; otherwise a number.
    fn keyword_or_number(&self, text: &[u8]) -> Result<TokenKind, LexError> {
        let Some((&first, rest)) = text.split_first() else {
            return self.keyword(text);
        };
        let rest = if first == b'-' {
            let dashes = rest.iter().take_while(|&&b| b == b'-').count();
            rest.get(dashes..).unwrap_or_default()
        } else {
            rest
        };
        let points = usize::from(first == b'.') + rest.iter().filter(|&&b| b == b'.').count();
        let damaged = rest
            .iter()
            .any(|&b| !(b.is_ascii_digit() || b == b'.' || b == b'-'));
        if damaged || points > 1 {
            return self.keyword(text);
        }
        Ok(lenient_number(text))
    }
}

/// A number in the standard's grammar (§7.3.3), or the offset within
/// `text` of the problem and the error.
fn strict_number(text: &[u8]) -> Result<TokenKind, (usize, LexErrorKind)> {
    let body = usize::from(matches!(text.first(), Some(b'+' | b'-')));
    let mut digits = 0usize;
    let mut point = false;
    for (at, &byte) in text.iter().enumerate().skip(body) {
        match byte {
            b'0'..=b'9' => digits += 1,
            b'.' if !point => point = true,
            _ => return Err((at, LexErrorKind::InvalidNumber)),
        }
    }
    if digits == 0 {
        return Err((0, LexErrorKind::InvalidNumber));
    }
    // Only an ASCII sign, digits and a point remain.
    let text = std::str::from_utf8(text).map_err(|_| (0, LexErrorKind::InvalidNumber))?;
    if !point {
        if let Ok(value) = text.parse::<i64>() {
            return Ok(TokenKind::Integer(value));
        }
    }
    // Past i64 an integer is the real its digits name. The standard
    // library's conversion is correctly rounded.
    match text.parse::<f64>() {
        Ok(value) if value.is_finite() => Ok(TokenKind::Real(value)),
        _ => Err((0, LexErrorKind::NumberOutOfRange)),
    }
}

/// Reads a damaged number: a leading run of `-` makes it negative, a `+`
/// is skipped, the leading digits are the integer part and, when the token
/// holds a `.`, the digits directly after a `.` that follows them are the
/// fraction. Values past binary64 become ±(largest finite binary64).
fn lenient_number(text: &[u8]) -> TokenKind {
    let dashes = text.iter().take_while(|&&b| b == b'-').count();
    let rest = text.get(dashes..).unwrap_or_default();
    let pluses = rest.iter().take_while(|&&b| b == b'+').count();
    let rest = rest.get(pluses..).unwrap_or_default();
    let whole_len = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    let (whole, after) = rest.split_at(whole_len);
    let whole = std::str::from_utf8(whole).unwrap_or_default();
    let sign = if dashes > 0 { "-" } else { "" };
    if !text.contains(&b'.') {
        if whole.is_empty() {
            return TokenKind::Integer(0);
        }
        let digits = format!("{sign}{whole}");
        if let Ok(value) = digits.parse::<i64>() {
            return TokenKind::Integer(value);
        }
        return TokenKind::Real(finite(digits.parse().unwrap_or(0.0)));
    }
    let fraction = match after.split_first() {
        Some((b'.', tail)) => {
            let len = tail.iter().take_while(|b| b.is_ascii_digit()).count();
            std::str::from_utf8(tail.get(..len).unwrap_or_default()).unwrap_or_default()
        }
        _ => "",
    };
    let whole = if whole.is_empty() { "0" } else { whole };
    let value = finite(format!("{sign}{whole}.{fraction}").parse().unwrap_or(0.0));
    // A negative zero reads as zero.
    TokenKind::Real(if value == 0.0 { 0.0 } else { value })
}

fn finite(value: f64) -> f64 {
    if value.is_finite() {
        value
    } else if value.is_sign_negative() {
        -f64::MAX
    } else {
        f64::MAX
    }
}
