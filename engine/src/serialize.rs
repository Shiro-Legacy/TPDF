// SPDX-License-Identifier: AGPL-3.0-or-later
//! PDF object syntax output (ISO 32000-2 §7.3) in two styles.
//!
//! - Canonical: the engine's own exact format. One space between tokens,
//!   strings in hex, dictionaries in key order, reals in positional notation
//!   with a point.
//! - Compact: what saved files use. White space only where two tokens
//!   would merge, the shorter string form, dictionaries in their own order
//!   with `/Type` then `/Subtype` first, and the shortest real that reads
//!   back exactly.

use crate::object::{Dictionary, Object, ObjectId};
use std::collections::BTreeSet;
use std::fmt;

/// Container nesting is capped here whatever the caller asks.
const MAX_DEPTH: usize = 256;

/// 2^63: integral reals below it in magnitude print as integers.
const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SerializeStyle {
    #[default]
    Canonical,
    Compact,
}

#[derive(Clone, Copy, Debug)]
pub struct WriteLimits {
    pub max_output_bytes: usize,
    pub max_depth: usize,
    pub max_objects: usize,
}

impl Default for WriteLimits {
    fn default() -> Self {
        Self {
            max_output_bytes: 16 << 20,
            max_depth: 64,
            max_objects: 100_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SerializeError {
    OutputLimit,
    DepthLimit,
    ObjectLimit,
    NullInName,
    NonfiniteReal,
    AllocationFailed,
}

impl fmt::Display for SerializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::OutputLimit => "PDF serialization output limit exceeded",
            Self::DepthLimit => "PDF serialization depth limit exceeded",
            Self::ObjectLimit => "PDF serialization object limit exceeded",
            Self::NullInName => "PDF names cannot contain null bytes",
            Self::NonfiniteReal => "PDF real numbers must be finite",
            Self::AllocationFailed => "PDF serialization allocation failed",
        })
    }
}

impl std::error::Error for SerializeError {}

/// What the last written byte means for the next token (compact style).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Last {
    /// Nothing, or bytes the caller appended: the caller separates.
    Caller,
    /// A delimiter: anything may follow directly.
    Delimiter,
    /// A regular character: a token starting with one needs a space.
    Regular,
    /// An empty name (`/`): a token starting with a regular character
    /// would join it.
    EmptyName,
}

pub(crate) struct Writer {
    out: Vec<u8>,
    limits: WriteLimits,
    style: SerializeStyle,
    keep_order: bool,
    nulls: BTreeSet<ObjectId>,
    objects: usize,
    last: Last,
}

impl Writer {
    pub(crate) fn with_style(limits: WriteLimits, style: SerializeStyle) -> Self {
        Self {
            out: Vec::new(),
            limits,
            style,
            keep_order: false,
            nulls: BTreeSet::new(),
            objects: 0,
            last: Last::Caller,
        }
    }

    /// Canonical dictionaries keep their own order instead of key order.
    #[allow(dead_code)] // The repair writer (workstream 03) calls it.
    pub(crate) fn keep_order(&mut self) {
        self.keep_order = true;
    }

    /// References to these objects are written as `null`.
    pub(crate) fn null_references(&mut self, ids: BTreeSet<ObjectId>) {
        self.nulls = ids;
    }

    pub(crate) fn len(&self) -> usize {
        self.out.len()
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.out
    }

    /// Raw bytes, counted toward the output limit.
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<(), SerializeError> {
        self.put(bytes)?;
        self.last = Last::Caller;
        Ok(())
    }

    pub(crate) fn value(&mut self, value: &Object, depth: usize) -> Result<(), SerializeError> {
        self.start_top_level();
        self.write_value(value, depth)
    }

    pub(crate) fn dictionary_entries<'a>(
        &mut self,
        entries: impl IntoIterator<Item = (&'a [u8], &'a Object)>,
    ) -> Result<(), SerializeError> {
        self.start_top_level();
        self.count()?;
        self.open(0)?;
        let entries: Vec<(&[u8], &Object)> = entries.into_iter().collect();
        let entries = match self.style {
            SerializeStyle::Canonical => entries,
            SerializeStyle::Compact => self.type_first(entries),
        };
        self.dictionary_body(&entries, 0)
    }

    /// A stream's dictionary with `/Length length`: an existing `/Length`
    /// keeps its place with the new value, a missing one goes last (or in
    /// key order in the canonical style).
    pub(crate) fn stream_dictionary(
        &mut self,
        values: &Dictionary,
        length: usize,
    ) -> Result<(), SerializeError> {
        self.start_top_level();
        self.count()?;
        self.open(0)?;
        let length = Object::Integer(i64::try_from(length).unwrap_or(i64::MAX));
        let mut entries: Vec<(&[u8], &Object)> = if self.sorts() {
            values
                .sorted_entries()
                .into_iter()
                .map(|(key, value)| (key.as_slice(), value))
                .collect()
        } else {
            values
                .iter()
                .map(|(key, value)| (key.as_slice(), value))
                .collect()
        };
        match entries.iter_mut().find(|(key, _)| *key == b"Length") {
            Some(entry) => entry.1 = &length,
            None if self.sorts() => {
                let at = entries.partition_point(|(key, _)| *key < b"Length".as_slice());
                entries.insert(at, (b"Length", &length));
            }
            None => entries.push((b"Length", &length)),
        }
        if self.style == SerializeStyle::Compact {
            entries = self.type_first(entries);
        }
        self.dictionary_body(&entries, 0)
    }

    fn compact(&self) -> bool {
        self.style == SerializeStyle::Compact
    }

    /// Canonical dictionaries are written in key order unless told to keep
    /// their own.
    fn sorts(&self) -> bool {
        self.style == SerializeStyle::Canonical && !self.keep_order
    }

    /// The first token of a top-level call takes no separator, except after
    /// an empty name, which any regular character would extend.
    fn start_top_level(&mut self) {
        if self.last == Last::Regular {
            self.last = Last::Caller;
        }
    }

    /// Makes room for `length` more output bytes: the output limit is
    /// checked first, so nothing is allocated for output that cannot fit.
    fn room(&mut self, length: usize) -> Result<(), SerializeError> {
        let total = self
            .out
            .len()
            .checked_add(length)
            .ok_or(SerializeError::OutputLimit)?;
        if total > self.limits.max_output_bytes {
            return Err(SerializeError::OutputLimit);
        }
        self.out
            .try_reserve(length)
            .map_err(|_| SerializeError::AllocationFailed)
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), SerializeError> {
        self.room(bytes.len())?;
        self.out.extend_from_slice(bytes);
        Ok(())
    }

    /// Writes a token, with a separating space in the compact style when it
    /// begins with a regular character that would join the previous token.
    fn token(&mut self, bytes: &[u8], last: Last) -> Result<(), SerializeError> {
        let regular_start = bytes
            .first()
            .is_some_and(|&b| !is_delimiter(b) && !is_white(b));
        if self.compact() && regular_start && matches!(self.last, Last::Regular | Last::EmptyName) {
            self.put(b" ")?;
        }
        self.put(bytes)?;
        self.last = last;
        Ok(())
    }

    /// A canonical-style separator.
    fn space(&mut self) -> Result<(), SerializeError> {
        if !self.compact() {
            self.put(b" ")?;
        }
        Ok(())
    }

    fn count(&mut self) -> Result<(), SerializeError> {
        self.objects += 1;
        if self.objects > self.limits.max_objects {
            return Err(SerializeError::ObjectLimit);
        }
        Ok(())
    }

    fn open(&self, depth: usize) -> Result<(), SerializeError> {
        if depth >= self.limits.max_depth.min(MAX_DEPTH) {
            return Err(SerializeError::DepthLimit);
        }
        Ok(())
    }

    fn write_value(&mut self, value: &Object, depth: usize) -> Result<(), SerializeError> {
        self.count()?;
        match value {
            Object::Null => self.token(b"null", Last::Regular),
            Object::Boolean(true) => self.token(b"true", Last::Regular),
            Object::Boolean(false) => self.token(b"false", Last::Regular),
            Object::Integer(value) => self.token(value.to_string().as_bytes(), Last::Regular),
            Object::Real(value) => {
                if !value.is_finite() {
                    return Err(SerializeError::NonfiniteReal);
                }
                let text = match self.style {
                    SerializeStyle::Canonical => canonical_real(*value),
                    SerializeStyle::Compact => compact_real(*value),
                };
                self.token(text.as_bytes(), Last::Regular)
            }
            Object::Reference(id) if self.nulls.contains(id) => self.token(b"null", Last::Regular),
            Object::Reference(id) => self.token(
                format!("{} {} R", id.number(), id.generation()).as_bytes(),
                Last::Regular,
            ),
            Object::Name(name) => self.name(name),
            Object::String(bytes) => self.string(bytes),
            Object::Array(items) => {
                self.open(depth)?;
                self.token(b"[", Last::Delimiter)?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        self.space()?;
                    }
                    self.write_value(item, depth + 1)?;
                }
                self.token(b"]", Last::Delimiter)
            }
            Object::Dictionary(dictionary) => {
                self.open(depth)?;
                let entries: Vec<(&[u8], &Object)> = if self.sorts() {
                    dictionary
                        .sorted_entries()
                        .into_iter()
                        .map(|(key, value)| (key.as_slice(), value))
                        .collect()
                } else {
                    dictionary
                        .iter()
                        .map(|(key, value)| (key.as_slice(), value))
                        .collect()
                };
                let entries = if self.compact() {
                    self.type_first(entries)
                } else {
                    entries
                };
                self.dictionary_body(&entries, depth)
            }
        }
    }

    /// `<<`, the entries with their values one level deeper, `>>`.
    fn dictionary_body(
        &mut self,
        entries: &[(&[u8], &Object)],
        depth: usize,
    ) -> Result<(), SerializeError> {
        self.token(b"<<", Last::Delimiter)?;
        for &(key, value) in entries {
            self.count()?;
            self.space()?;
            self.name(key)?;
            self.space()?;
            self.write_value(value, depth + 1)?;
        }
        self.space()?;
        self.token(b">>", Last::Delimiter)
    }

    /// A name (§7.3.5): bytes outside `!`..`~`, delimiters and `#` are
    /// written as `#` and two uppercase hex digits.
    fn name(&mut self, name: &[u8]) -> Result<(), SerializeError> {
        if name.contains(&0) {
            return Err(SerializeError::NullInName);
        }
        let plain = |byte: u8| (0x21..=0x7E).contains(&byte) && !is_delimiter(byte) && byte != b'#';
        let length = name.iter().fold(1usize, |length, &byte| {
            length.saturating_add(if plain(byte) { 1 } else { 3 })
        });
        // A name begins with a delimiter, so it never takes a separator.
        self.room(length)?;
        self.out.push(b'/');
        for &byte in name {
            if plain(byte) {
                self.out.push(byte);
            } else {
                self.out.push(b'#');
                self.out.extend_from_slice(&hex_pair(byte));
            }
        }
        self.last = if name.is_empty() {
            Last::EmptyName
        } else {
            Last::Regular
        };
        Ok(())
    }

    /// A string: hex in the canonical style; in the compact style the
    /// literal form unless it would be longer than the hex form, or the
    /// string starts with a UTF-16 byte order mark. The length is known
    /// before anything is allocated.
    fn string(&mut self, bytes: &[u8]) -> Result<(), SerializeError> {
        let hex_length = bytes.len().saturating_mul(2).saturating_add(2);
        let literal = self.compact() && {
            let marked = bytes.starts_with(&[0xFE, 0xFF]) || bytes.starts_with(&[0xFF, 0xFE]);
            !marked && literal_length(bytes) <= hex_length
        };
        // A string begins with a delimiter, so it never takes a separator.
        if literal {
            self.room(literal_length(bytes))?;
            write_literal(&mut self.out, bytes);
        } else {
            self.room(hex_length)?;
            write_hex(&mut self.out, bytes);
        }
        self.last = Last::Delimiter;
        Ok(())
    }

    /// Whether a value counts as null for the `/Type`, `/Subtype` order.
    fn prints_null(&self, value: &Object) -> bool {
        match value {
            Object::Null => true,
            Object::Reference(id) => self.nulls.contains(id),
            _ => false,
        }
    }

    /// Compact order: a non-null `/Type` first, then (only when `/Type`
    /// moved) a non-null `/Subtype`; the others keep their order.
    fn type_first<'a>(&self, entries: Vec<(&'a [u8], &'a Object)>) -> Vec<(&'a [u8], &'a Object)> {
        let find = |name: &[u8]| {
            entries
                .iter()
                .position(|(key, value)| *key == name && !self.prints_null(value))
        };
        let Some(type_at) = find(b"Type") else {
            return entries;
        };
        let subtype_at = find(b"Subtype");
        let mut ordered = Vec::with_capacity(entries.len());
        ordered.push(entries[type_at]);
        if let Some(at) = subtype_at {
            ordered.push(entries[at]);
        }
        ordered.extend(
            entries
                .iter()
                .enumerate()
                .filter(|(at, _)| *at != type_at && Some(*at) != subtype_at)
                .map(|(_, entry)| *entry),
        );
        ordered
    }
}

fn is_white(byte: u8) -> bool {
    matches!(byte, 0x00 | 0x09 | 0x0A | 0x0C | 0x0D | 0x20)
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn hex_pair(byte: u8) -> [u8; 2] {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    [
        DIGITS[usize::from(byte >> 4)],
        DIGITS[usize::from(byte & 0x0F)],
    ]
}

/// `<…>` with two uppercase digits per byte, into room already reserved.
fn write_hex(out: &mut Vec<u8>, bytes: &[u8]) {
    out.push(b'<');
    for &byte in bytes {
        out.extend_from_slice(&hex_pair(byte));
    }
    out.push(b'>');
}

/// The bytes a literal string spends on `byte`.
fn literal_cost(byte: u8) -> usize {
    match byte {
        b'\n' | b'\r' | b'\t' | 0x08 | 0x0C | b'(' | b')' | b'\\' => 2,
        0x00..=0x1F | 0x7F..=0xFF => 4,
        _ => 1,
    }
}

/// The length of the literal form of `bytes`, parentheses included.
fn literal_length(bytes: &[u8]) -> usize {
    bytes.iter().fold(2usize, |length, &byte| {
        length.saturating_add(literal_cost(byte))
    })
}

/// `(…)` with escapes, into room already reserved.
fn write_literal(out: &mut Vec<u8>, bytes: &[u8]) {
    out.push(b'(');
    for &byte in bytes {
        match byte {
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x0C => out.extend_from_slice(b"\\f"),
            b'(' | b')' | b'\\' => out.extend_from_slice(&[b'\\', byte]),
            0x00..=0x1F | 0x7F..=0xFF => {
                out.extend_from_slice(&[
                    b'\\',
                    b'0' + (byte >> 6),
                    b'0' + ((byte >> 3) & 7),
                    b'0' + (byte & 7),
                ]);
            }
            _ => out.push(byte),
        }
    }
    out.push(b')');
}

/// The shortest decimal digits that read back as `value`, positionally.
/// The standard library's `Display` for `f64` prints exactly that: the
/// shortest round-trip digit string, without an exponent.
fn shortest(value: f64) -> String {
    format!("{value}")
}

/// Canonical reals: the shortest round-trip digits, positional, with a
/// point (`.0` added when there is none) and a leading `0` below one.
fn canonical_real(value: f64) -> String {
    let mut text = shortest(value);
    if !text.contains('.') {
        text.push_str(".0");
    }
    text
}

/// Compact reals: integral values below 2^63 as integers; otherwise the
/// shortest round-trip digits without a `0` before the point (and with a
/// trailing `.` for integral values, so they read back as reals). NaN
/// gives `0` and the infinities the largest finite values.
pub(crate) fn compact_real(value: f64) -> String {
    if value.is_nan() {
        return "0".into();
    }
    let value = if value.is_infinite() {
        f64::MAX.copysign(value)
    } else {
        value
    };
    if value.fract() == 0.0 && (-TWO_POW_63..TWO_POW_63).contains(&value) {
        // In range, so the conversion is exact.
        return (value as i64).to_string();
    }
    let text = shortest(value);
    if !text.contains('.') {
        return text + ".";
    }
    if let Some(rest) = text.strip_prefix("-0.") {
        return format!("-.{rest}");
    }
    if let Some(rest) = text.strip_prefix("0.") {
        return format!(".{rest}");
    }
    text
}

pub fn serialize_object(value: &Object, limits: WriteLimits) -> Result<Vec<u8>, SerializeError> {
    serialize_object_with_style(value, limits, SerializeStyle::Canonical)
}

pub fn serialize_object_with_style(
    value: &Object,
    limits: WriteLimits,
    style: SerializeStyle,
) -> Result<Vec<u8>, SerializeError> {
    let mut writer = Writer::with_style(limits, style);
    writer.value(value, 0)?;
    Ok(writer.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An over-limit string (literal or hex) or name fails with the output
    /// limit before the output grows to its size: the check comes first.
    #[test]
    fn over_limit_strings_and_names_fail_before_allocating() {
        const LIMIT: usize = 64;
        let limits = WriteLimits {
            max_output_bytes: LIMIT,
            ..WriteLimits::default()
        };
        let big = 1 << 20;
        let cases = [
            (Object::String(vec![b'a'; big]), SerializeStyle::Compact),
            (Object::String(vec![0; big]), SerializeStyle::Compact),
            (Object::String(vec![b'a'; big]), SerializeStyle::Canonical),
            (Object::Name(vec![b'a'; big]), SerializeStyle::Compact),
            (Object::Name(vec![b' '; big]), SerializeStyle::Canonical),
        ];
        for (value, style) in cases {
            let mut writer = Writer::with_style(limits, style);
            assert_eq!(
                writer.value(&value, 0),
                Err(SerializeError::OutputLimit),
                "{style:?}"
            );
            assert!(
                writer.len() == 0 && writer.out.capacity() <= LIMIT,
                "{style:?}"
            );
            assert_eq!(
                serialize_object_with_style(&value, limits, style),
                Err(SerializeError::OutputLimit)
            );
        }
        // An exact fit still succeeds for each form.
        for (value, style, text) in [
            (
                Object::String(b"ab".to_vec()),
                SerializeStyle::Compact,
                &b"(ab)"[..],
            ),
            (
                Object::String(vec![0, 1]),
                SerializeStyle::Compact,
                b"<0001>",
            ),
            (
                Object::String(vec![0xFF]),
                SerializeStyle::Canonical,
                b"<FF>",
            ),
            (
                Object::Name(b"a b".to_vec()),
                SerializeStyle::Compact,
                b"/a#20b",
            ),
        ] {
            let exact = WriteLimits {
                max_output_bytes: text.len(),
                ..limits
            };
            assert_eq!(
                serialize_object_with_style(&value, exact, style).as_deref(),
                Ok(text)
            );
            let short = WriteLimits {
                max_output_bytes: text.len() - 1,
                ..limits
            };
            assert_eq!(
                serialize_object_with_style(&value, short, style),
                Err(SerializeError::OutputLimit)
            );
        }
    }
}
