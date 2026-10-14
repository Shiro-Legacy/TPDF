// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded decoding through an explicit ToUnicode map or simple encoding.
//!
//! This preserves original character-code bytes and per-code output spans. It
//! does not load fonts, infer reading order, replace missing mappings, normalize
//! Unicode or interpret an encoding CMap as Unicode. Errors return no partial
//! result. The original input is borrowed; the result does not borrow the map.

use crate::cmap::{CharacterCode, CodeDecodeError, CodeWidth, ToUnicodeMap};
use crate::simple_encoding::{GlyphUnicode, SimpleEncoding};
use std::{error::Error, fmt, ops::Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextDecodeLimits {
    pub max_input_bytes: usize,
    pub max_codes: usize,
    pub max_scalars: usize,
    pub max_utf8_bytes: usize,
}
impl Default for TextDecodeLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 1024 * 1024,
            max_codes: 262_144,
            max_scalars: 1_000_000,
            max_utf8_bytes: 4 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedCode {
    code: CharacterCode,
    source_span: Range<usize>,
    utf8_span: Range<usize>,
}
impl DecodedCode {
    pub fn code(&self) -> CharacterCode {
        self.code
    }
    pub fn source_span(&self) -> Range<usize> {
        self.source_span.clone()
    }
    pub fn utf8_span(&self) -> Range<usize> {
        self.utf8_span.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedText<'input> {
    input: &'input [u8],
    text: String,
    codes: Vec<DecodedCode>,
    scalar_count: usize,
}
impl<'input> DecodedText<'input> {
    pub fn input(&self) -> &'input [u8] {
        self.input
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn codes(&self) -> &[DecodedCode] {
        &self.codes
    }
    pub fn scalar_count(&self) -> usize {
        self.scalar_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextDecodeLimitKind {
    InputBytes,
    Codes,
    Scalars,
    Utf8Bytes,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextDecodeErrorKind {
    MissingMapping(CharacterCode),
    UndefinedEncodingCode(u8),
    UnknownGlyphName(u8),
    InvalidCode,
    IncompleteCode,
    LimitExceeded(TextDecodeLimitKind),
    AllocationFailed,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDecodeError {
    pub offset: usize,
    pub kind: TextDecodeErrorKind,
}
impl fmt::Display for TextDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "mapped text error at source byte {}: {:?}",
            self.offset, self.kind
        )
    }
}
impl Error for TextDecodeError {}
fn limit(offset: usize, kind: TextDecodeLimitKind) -> TextDecodeError {
    TextDecodeError {
        offset,
        kind: TextDecodeErrorKind::LimitExceeded(kind),
    }
}
fn allocation(offset: usize) -> TextDecodeError {
    TextDecodeError {
        offset,
        kind: TextDecodeErrorKind::AllocationFailed,
    }
}
fn growth(current: usize, required: usize, limit: usize) -> usize {
    // Callers first establish required <= limit. Doubling is amortized linear;
    // overflow saturates at the caller's cap, never wrapping a capacity request.
    current
        .checked_mul(2)
        .unwrap_or(limit)
        .min(limit)
        .max(required)
}

/// Decode through an authoritative ToUnicode map. Missing entries never fall back.
pub fn decode_mapped_text<'input>(
    map: &ToUnicodeMap,
    input: &'input [u8],
    limits: TextDecodeLimits,
) -> Result<DecodedText<'input>, TextDecodeError> {
    decode_codes(input, limits, |remaining| {
        let (code, consumed) = map.decode_code_prefix(remaining).map_err(|e| match e {
            CodeDecodeError::EmptyInput | CodeDecodeError::IncompleteCode => {
                TextDecodeErrorKind::IncompleteCode
            }
            CodeDecodeError::OutsideCodeSpace => TextDecodeErrorKind::InvalidCode,
        })?;
        let sequence = map
            .lookup(code)
            .ok_or(TextDecodeErrorKind::MissingMapping(code))?;
        Ok((code, consumed, sequence.scalars()))
    })
}

/// Decode declared simple-encoding names. This establishes no font-program glyph
/// equivalence. Undefined slots and unknown glyph names are distinct failures.
pub fn decode_encoded_text<'input>(
    encoding: &SimpleEncoding,
    input: &'input [u8],
    limits: TextDecodeLimits,
) -> Result<DecodedText<'input>, TextDecodeError> {
    decode_codes(input, limits, |remaining| {
        let byte = remaining[0]; // private accumulator only calls with nonempty input
        let code = CharacterCode::new(u32::from(byte), CodeWidth::One)
            .expect("a byte fits a one-byte character code");
        match encoding.lookup_unicode(byte) {
            GlyphUnicode::Unicode(scalars) => Ok((code, 1, scalars)),
            GlyphUnicode::UndefinedCode => Err(TextDecodeErrorKind::UndefinedEncodingCode(byte)),
            GlyphUnicode::UnknownGlyphName(_) => Err(TextDecodeErrorKind::UnknownGlyphName(byte)),
        }
    })
}

// Both lookup policies share the same checked allocation, cumulative work and
// span publication. Mapping slices borrow the immutable policy, not the input.
fn decode_codes<'input, 'mapping>(
    input: &'input [u8],
    limits: TextDecodeLimits,
    mut lookup: impl FnMut(
        &[u8],
    ) -> Result<(CharacterCode, usize, &'mapping [char]), TextDecodeErrorKind>,
) -> Result<DecodedText<'input>, TextDecodeError> {
    if input.len() > limits.max_input_bytes {
        return Err(limit(0, TextDecodeLimitKind::InputBytes));
    }
    let mut result = DecodedText {
        input,
        text: String::new(),
        codes: Vec::new(),
        scalar_count: 0,
    };
    let mut offset = 0usize;
    while offset < input.len() {
        if result.codes.len() >= limits.max_codes {
            return Err(limit(offset, TextDecodeLimitKind::Codes));
        }
        let (code, consumed, sequence) =
            lookup(&input[offset..]).map_err(|kind| TextDecodeError { offset, kind })?;
        let new_scalars = result
            .scalar_count
            .checked_add(sequence.len())
            .filter(|n| *n <= limits.max_scalars)
            .ok_or_else(|| limit(offset, TextDecodeLimitKind::Scalars))?;
        let new_bytes = sequence.iter().try_fold(result.text.len(), |n, scalar| {
            n.checked_add(scalar.len_utf8())
                .filter(|n| *n <= limits.max_utf8_bytes)
                .ok_or_else(|| limit(offset, TextDecodeLimitKind::Utf8Bytes))
        })?;
        let source_end = offset
            .checked_add(consumed)
            .filter(|end| consumed > 0 && *end <= input.len())
            .ok_or(TextDecodeError {
                offset,
                kind: TextDecodeErrorKind::InvalidCode,
            })?;
        let code_count = result
            .codes
            .len()
            .checked_add(1)
            .ok_or_else(|| limit(offset, TextDecodeLimitKind::Codes))?;
        if new_bytes > result.text.capacity() {
            let capacity = growth(result.text.capacity(), new_bytes, limits.max_utf8_bytes);
            result
                .text
                .try_reserve_exact(capacity - result.text.len())
                .map_err(|_| allocation(offset))?;
        }
        if code_count > result.codes.capacity() {
            let capacity = growth(result.codes.capacity(), code_count, limits.max_codes);
            result
                .codes
                .try_reserve_exact(capacity - result.codes.len())
                .map_err(|_| allocation(offset))?;
        }
        let utf8_start = result.text.len();
        for scalar in sequence {
            result.text.push(*scalar);
        }
        result.codes.push(DecodedCode {
            code,
            source_span: offset..source_end,
            utf8_span: utf8_start..new_bytes,
        });
        result.scalar_count = new_scalars;
        offset = source_end;
    }
    Ok(result)
}
