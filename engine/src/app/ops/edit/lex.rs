// SPDX-License-Identifier: AGPL-3.0-or-later
//! The edit scanner's content tokenizer (content-edit.ts `nextToken`): byte
//! spans for every token, so a rewrite splices the decoded stream exactly.
//! `true`, `false` and `null` are operands here; engine.ts lexed them as
//! operators, which refused a page with a boolean in a `BDC` dictionary.
use std::ops::Range;

use super::{MAX_EDIT_NESTING, MAX_EDIT_TOKENS, scan_error};
use crate::app::AppError;

pub fn is_white(byte: Option<u8>) -> bool {
    matches!(byte, Some(0 | 9 | 10 | 12 | 13 | 32))
}

pub fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

pub fn is_regular(byte: u8) -> bool {
    !is_white(Some(byte)) && !is_delimiter(byte)
}

fn hex_digit(byte: Option<u8>) -> Option<u8> {
    match byte? {
        b @ b'0'..=b'9' => Some(b - b'0'),
        b @ b'A'..=b'F' => Some(b - b'A' + 10),
        b @ b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

/// One operand with its byte span.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Num(f64, Range<usize>),
    Name(Vec<u8>, Range<usize>),
    Str(Vec<u8>, Range<usize>),
    Arr(Vec<Operand>, Range<usize>),
    /// A dictionary: only its span is kept (a `BDC` list is re-lexed).
    Dict(Range<usize>),
    /// `true`, `false` or `null`.
    Word(Range<usize>),
}

impl Operand {
    pub fn span(&self) -> Range<usize> {
        match self {
            Self::Num(_, span)
            | Self::Name(_, span)
            | Self::Str(_, span)
            | Self::Arr(_, span)
            | Self::Dict(span)
            | Self::Word(span) => span.clone(),
        }
    }
    pub fn number(&self) -> Option<f64> {
        match self {
            Self::Num(n, _) => Some(*n),
            _ => None,
        }
    }
    pub fn name(&self) -> Option<&[u8]> {
        match self {
            Self::Name(name, _) => Some(name),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Array,
    Dict,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Token {
    Operand(Operand),
    Open(Container, Range<usize>),
    Close(Container, Range<usize>),
    Op(Range<usize>),
}

/// A cursor over one decoded stream; `tokens` counts toward the page budget.
pub struct Lexer<'b> {
    pub bytes: &'b [u8],
    pub pos: usize,
}

/// `/^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/`.
fn is_number(run: &[u8]) -> bool {
    let mut i = usize::from(matches!(run.first(), Some(b'+' | b'-')));
    let digits = |i: &mut usize| {
        let start = *i;
        while run.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - start
    };
    if digits(&mut i) > 0 {
        if run.get(i) == Some(&b'.') {
            i += 1;
            digits(&mut i);
        }
    } else if run.get(i) == Some(&b'.') {
        i += 1;
        if digits(&mut i) == 0 {
            return false;
        }
    } else {
        return false;
    }
    if matches!(run.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(run.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == run.len()
}

impl<'b> Lexer<'b> {
    pub fn new(bytes: &'b [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn at(&self, index: usize) -> Option<u8> {
        self.bytes.get(index).copied()
    }

    /// The next token, None at the end. `tokens` is the page's token count.
    pub fn next(&mut self, tokens: &mut usize) -> Result<Option<Token>, AppError> {
        loop {
            while self.pos < self.bytes.len() && is_white(self.at(self.pos)) {
                self.pos += 1;
            }
            if self.pos >= self.bytes.len() {
                return Ok(None);
            }
            if self.at(self.pos) == Some(b'%') {
                while self.pos < self.bytes.len() && !matches!(self.at(self.pos), Some(10 | 13)) {
                    self.pos += 1;
                }
                continue;
            }
            break;
        }
        *tokens += 1;
        if *tokens > MAX_EDIT_TOKENS {
            return Err(scan_error(
                "too-large",
                format!("edit-targets: more than {MAX_EDIT_TOKENS} tokens on this page"),
            ));
        }
        let start = self.pos;
        let Some(byte) = self.at(start) else {
            return Ok(None);
        };
        match byte {
            b'[' => {
                self.pos += 1;
                return Ok(Some(Token::Open(Container::Array, start..self.pos)));
            }
            b']' => {
                self.pos += 1;
                return Ok(Some(Token::Close(Container::Array, start..self.pos)));
            }
            b'<' if self.at(start + 1) == Some(b'<') => {
                self.pos += 2;
                return Ok(Some(Token::Open(Container::Dict, start..self.pos)));
            }
            b'>' if self.at(start + 1) == Some(b'>') => {
                self.pos += 2;
                return Ok(Some(Token::Close(Container::Dict, start..self.pos)));
            }
            b'<' => return self.hex(start).map(Some),
            b'>' => {
                return Err(scan_error(
                    "invalid",
                    "edit-targets: an unexpected '>' in a content stream",
                ));
            }
            b'(' => return self.literal(start).map(Some),
            b'/' => {
                self.pos += 1;
                let mut out = Vec::new();
                while let Some(b) = self.at(self.pos).filter(|b| is_regular(*b)) {
                    if b == b'#' {
                        if let (Some(high), Some(low)) = (
                            hex_digit(self.at(self.pos + 1)),
                            hex_digit(self.at(self.pos + 2)),
                        ) {
                            out.push((high << 4) | low);
                            self.pos += 3;
                            continue;
                        }
                    }
                    out.push(b);
                    self.pos += 1;
                }
                return Ok(Some(Token::Operand(Operand::Name(out, start..self.pos))));
            }
            _ => {}
        }
        while self.at(self.pos).is_some_and(is_regular) {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(scan_error(
                "invalid",
                format!("edit-targets: an unexpected byte 0x{byte:x} in a content stream"),
            ));
        }
        let run = &self.bytes[start..self.pos];
        if is_number(run) {
            let text = String::from_utf8_lossy(run);
            let value = text.parse::<f64>().unwrap_or(f64::NAN);
            return Ok(Some(Token::Operand(Operand::Num(value, start..self.pos))));
        }
        if matches!(run, b"true" | b"false" | b"null") {
            return Ok(Some(Token::Operand(Operand::Word(start..self.pos))));
        }
        Ok(Some(Token::Op(start..self.pos)))
    }

    fn hex(&mut self, start: usize) -> Result<Token, AppError> {
        self.pos += 1;
        let mut out = Vec::new();
        let mut high: Option<u8> = None;
        loop {
            let Some(next) = self.at(self.pos) else {
                return Err(scan_error(
                    "invalid",
                    "edit-targets: an unterminated hex string",
                ));
            };
            if is_white(Some(next)) {
                self.pos += 1;
                continue;
            }
            if next == b'>' {
                self.pos += 1;
                if let Some(high) = high {
                    out.push(high << 4);
                }
                return Ok(Token::Operand(Operand::Str(out, start..self.pos)));
            }
            let Some(digit) = hex_digit(Some(next)) else {
                return Err(scan_error(
                    "invalid",
                    "edit-targets: a hex string holds a non-hex character",
                ));
            };
            self.pos += 1;
            match high.take() {
                None => high = Some(digit),
                Some(h) => out.push((h << 4) | digit),
            }
        }
    }

    fn literal(&mut self, start: usize) -> Result<Token, AppError> {
        let unterminated = || scan_error("invalid", "edit-targets: an unterminated literal string");
        self.pos += 1;
        let mut out = Vec::new();
        let mut nesting = 1usize;
        loop {
            let Some(next) = self.at(self.pos) else {
                return Err(unterminated());
            };
            match next {
                b'\\' => {
                    self.pos += 1;
                    let Some(escaped) = self.at(self.pos) else {
                        return Err(unterminated());
                    };
                    match escaped {
                        b'0'..=b'7' => {
                            let mut value = 0u32;
                            let mut digits = 0;
                            while digits < 3 {
                                match self.at(self.pos) {
                                    Some(d @ b'0'..=b'7') => {
                                        value = value * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                        digits += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push((value & 0xff) as u8);
                            continue;
                        }
                        b'n' => out.push(10),
                        b'r' => out.push(13),
                        b't' => out.push(9),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        10 => {}
                        13 => {
                            self.pos += 1;
                            if self.at(self.pos) == Some(10) {
                                self.pos += 1;
                            }
                            continue;
                        }
                        other => out.push(other),
                    }
                    self.pos += 1;
                }
                b'(' => {
                    nesting += 1;
                    if nesting > MAX_EDIT_NESTING {
                        return Err(scan_error(
                            "too-large",
                            format!("edit-targets: literal-string nesting over {MAX_EDIT_NESTING}"),
                        ));
                    }
                    out.push(next);
                    self.pos += 1;
                }
                b')' => {
                    nesting -= 1;
                    self.pos += 1;
                    if nesting == 0 {
                        return Ok(Token::Operand(Operand::Str(out, start..self.pos)));
                    }
                    out.push(next);
                }
                _ => {
                    out.push(next);
                    self.pos += 1;
                }
            }
        }
    }
}

/// content-edit.ts `editNextQEnd`: the end of a `Q` that follows `from`
/// with only whitespace and comments between, else None.
pub fn next_q_end(bytes: &[u8], from: usize) -> Option<usize> {
    let mut position = from;
    loop {
        while position < bytes.len() && is_white(bytes.get(position).copied()) {
            position += 1;
        }
        if bytes.get(position) != Some(&b'%') {
            break;
        }
        while position < bytes.len() && !matches!(bytes.get(position), Some(10 | 13)) {
            position += 1;
        }
    }
    if bytes.get(position) != Some(&b'Q') {
        return None;
    }
    let end = position + 1;
    if let Some(&next) = bytes.get(end) {
        if !is_white(Some(next)) && !is_delimiter(next) {
            return None;
        }
    }
    Some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(bytes: &[u8]) -> Vec<Token> {
        let mut lexer = Lexer::new(bytes);
        let mut tokens = 0;
        let mut out = Vec::new();
        while let Some(token) = lexer.next(&mut tokens).unwrap() {
            out.push(token);
        }
        out
    }

    #[test]
    fn numbers_names_strings_and_words() {
        let tokens = all(b"1 -2.5 .5 1e3 /A#20b (a\\(b\\101) <4142 3> true Tj");
        assert_eq!(tokens[0], Token::Operand(Operand::Num(1.0, 0..1)));
        assert_eq!(tokens[1], Token::Operand(Operand::Num(-2.5, 2..6)));
        assert_eq!(tokens[2], Token::Operand(Operand::Num(0.5, 7..9)));
        assert_eq!(tokens[3], Token::Operand(Operand::Num(1000.0, 10..13)));
        assert_eq!(
            tokens[4],
            Token::Operand(Operand::Name(b"A b".to_vec(), 14..20))
        );
        assert_eq!(
            tokens[5],
            Token::Operand(Operand::Str(b"a(bA".to_vec(), 21..31))
        );
        assert_eq!(
            tokens[6],
            Token::Operand(Operand::Str(b"AB0".to_vec(), 32..40))
        );
        assert_eq!(tokens[7], Token::Operand(Operand::Word(41..45)));
        assert_eq!(tokens[8], Token::Op(46..48));
    }

    #[test]
    fn q_end_after_comments() {
        assert_eq!(next_q_end(b"Do % x\n Q", 2), Some(9));
        assert_eq!(next_q_end(b"Do Qx", 2), None);
    }
}
