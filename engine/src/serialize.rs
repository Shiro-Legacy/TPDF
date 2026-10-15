// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded serialization of owned PDF objects, not whole documents.
use crate::object::{Dictionary, Object, ObjectId, SORT_AFTER};
use std::collections::BTreeSet;
use std::fmt;

/// How objects are printed. `Canonical` separates every token with a space,
/// writes strings as hex and dictionary keys in byte order. `Compact` follows
/// MuPDF's tight printer (`pdf_print_obj` with tight=1): a space only between
/// two tokens that are not delimiters; dictionary keys in the dictionary's
/// order, except that a non-null /Type comes first, followed by /Subtype when
/// that is non-null too; literal strings unless they start with a UTF-16 byte
/// order mark or escaping would double their length; and reals as MuPDF
/// prints a float.
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
            max_output_bytes: 16 * 1024 * 1024,
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

pub(crate) struct Writer {
    bytes: Vec<u8>,
    limits: WriteLimits,
    objects: usize,
    style: SerializeStyle,
    /// Compact style: the previous token asked for a separator.
    separate: bool,
    /// References printed as null (targets that are not written).
    nulls: BTreeSet<ObjectId>,
    /// Canonical style: dictionary keys in the dictionary's order, not byte
    /// order.
    keep_order: bool,
}
impl Writer {
    pub(crate) fn with_style(limits: WriteLimits, style: SerializeStyle) -> Self {
        Self {
            bytes: Vec::new(),
            limits,
            objects: 0,
            style,
            separate: false,
            nulls: BTreeSet::new(),
            keep_order: false,
        }
    }
    /// Print canonical dictionaries in the dictionary's order (repair output).
    pub(crate) fn keep_order(&mut self) {
        self.keep_order = true;
    }
    pub(crate) fn null_references(&mut self, ids: BTreeSet<ObjectId>) {
        self.nulls = ids;
    }
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }
    pub(crate) fn finish(self) -> Vec<u8> {
        self.bytes
    }
    /// Emit a stream dictionary without cloning any values or decoded payloads.
    /// /Length gets the actual byte count, placed as MuPDF's `pdf_dict_put`
    /// places it: in place, or where a new key goes, and a dictionary that
    /// holds more than 100 entries with /Length is printed sorted, as that put
    /// sorts it; in byte order in the canonical style.
    pub(crate) fn stream_dictionary(
        &mut self,
        values: &Dictionary,
        length: usize,
    ) -> Result<(), SerializeError> {
        if self.style == SerializeStyle::Compact || self.keep_order {
            let length =
                Object::Integer(i64::try_from(length).map_err(|_| SerializeError::OutputLimit)?);
            let has_length = values.contains_key(b"Length".as_slice());
            let sorted = values.is_sorted()
                || values.len().saturating_add(usize::from(!has_length)) > SORT_AFTER;
            let mut entries: Vec<(&[u8], &Object)> = values
                .iter()
                .map(|(key, value)| match key.as_slice() {
                    b"Length" => (key.as_slice(), &length),
                    key => (key, value),
                })
                .collect();
            if sorted && !values.is_sorted() {
                entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
            }
            if !has_length {
                let at = if sorted {
                    entries.partition_point(|(key, _)| *key < b"Length".as_slice())
                } else {
                    entries.len()
                };
                entries.insert(at, (b"Length", &length));
            }
            return self.dictionary_entries(entries);
        }
        self.object_charge()?;
        if self.limits.max_depth == 0 {
            return Err(SerializeError::DepthLimit);
        }
        let length = i64::try_from(length).map_err(|_| SerializeError::OutputLimit)?;
        self.append(b"<<")?;
        let mut emitted = false;
        for (key, value) in values.sorted_entries() {
            if !emitted && key.as_slice() >= b"Length" {
                self.object_charge()?;
                self.append(b" /Length ")?;
                self.value(&Object::Integer(length), 1)?;
                emitted = true;
            }
            if key == b"Length" {
                continue;
            }
            self.object_charge()?;
            self.append(b" ")?;
            self.name(key)?;
            self.append(b" ")?;
            self.value(value, 1)?;
        }
        if !emitted {
            self.object_charge()?;
            self.append(b" /Length ")?;
            self.value(&Object::Integer(length), 1)?;
        }
        self.append(b" >>")
    }

    pub(crate) fn dictionary_entries<'a>(
        &mut self,
        entries: impl IntoIterator<Item = (&'a [u8], &'a Object)>,
    ) -> Result<(), SerializeError> {
        self.object_charge()?;
        if self.limits.max_depth == 0 {
            return Err(SerializeError::DepthLimit);
        }
        if self.style == SerializeStyle::Compact {
            return self.compact_dictionary(entries, 0);
        }
        self.append(b"<<")?;
        for (key, value) in entries {
            self.object_charge()?;
            self.append(b" ")?;
            self.name(key)?;
            self.append(b" ")?;
            self.value(value, 1)?;
        }
        self.append(b" >>")
    }
    fn reserve(&mut self, extra: usize) -> Result<(), SerializeError> {
        let end = self
            .bytes
            .len()
            .checked_add(extra)
            .filter(|&n| n <= self.limits.max_output_bytes)
            .ok_or(SerializeError::OutputLimit)?;
        if end > self.bytes.capacity() {
            let target = end
                .max(self.bytes.capacity().saturating_mul(2).max(64))
                .min(self.limits.max_output_bytes);
            self.bytes
                .try_reserve_exact(target - self.bytes.len())
                .map_err(|_| SerializeError::AllocationFailed)?;
        }
        Ok(())
    }
    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<(), SerializeError> {
        self.reserve(bytes.len())?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn object_charge(&mut self) -> Result<(), SerializeError> {
        self.objects = self
            .objects
            .checked_add(1)
            .filter(|&n| n <= self.limits.max_objects)
            .ok_or(SerializeError::ObjectLimit)?;
        Ok(())
    }
    fn name(&mut self, name: &[u8]) -> Result<(), SerializeError> {
        if name.contains(&0) {
            return Err(SerializeError::NullInName);
        }
        let length = name
            .iter()
            .try_fold(1usize, |n, &b| {
                n.checked_add(if plain_name(b) { 1 } else { 3 })
            })
            .ok_or(SerializeError::OutputLimit)?;
        self.reserve(length)?;
        self.bytes.push(b'/');
        for &byte in name {
            if plain_name(byte) {
                self.bytes.push(byte);
            } else {
                self.bytes.extend_from_slice(&[
                    b'#',
                    HEX[(byte >> 4) as usize],
                    HEX[(byte & 15) as usize],
                ]);
            }
        }
        Ok(())
    }
    /// A token after an optional separator; names, strings and containers
    /// start with a delimiter, so they never take one.
    fn token(&mut self, token: &[u8]) -> Result<(), SerializeError> {
        let delimiter = |byte: u8| b"()<>[]{}/%".contains(&byte);
        if std::mem::take(&mut self.separate)
            && self.bytes.last().is_some_and(|&b| !delimiter(b))
            && token.first().is_some_and(|&b| !delimiter(b))
        {
            self.append(b" ")?;
        }
        self.append(token)
    }
    fn compact(&mut self, value: &Object, depth: usize) -> Result<(), SerializeError> {
        self.object_charge()?;
        match value {
            Object::Null => self.token(b"null"),
            Object::Boolean(true) => self.token(b"true"),
            Object::Boolean(false) => self.token(b"false"),
            Object::Integer(value) => self.token(value.to_string().as_bytes()),
            Object::Real(value) => {
                if !value.is_finite() {
                    return Err(SerializeError::NonfiniteReal);
                }
                self.token(compact_real(*value).as_bytes())
            }
            Object::Reference(id) if self.nulls.contains(id) => self.token(b"null"),
            Object::Reference(id) => {
                self.token(format!("{} {} R", id.number(), id.generation()).as_bytes())
            }
            Object::Name(name) => {
                self.separate = false;
                self.name(name)
            }
            Object::String(bytes) => {
                self.separate = false;
                self.compact_string(bytes)
            }
            Object::Array(values) => {
                if depth >= self.limits.max_depth.min(256) {
                    return Err(SerializeError::DepthLimit);
                }
                self.separate = false;
                self.append(b"[")?;
                for value in values {
                    self.compact(value, depth + 1)?;
                    self.separate = true;
                }
                self.separate = false;
                self.append(b"]")
            }
            Object::Dictionary(values) => {
                if depth >= self.limits.max_depth.min(256) {
                    return Err(SerializeError::DepthLimit);
                }
                self.compact_dictionary(values.iter().map(|(k, v)| (k.as_slice(), v)), depth)
            }
        }
    }
    /// MuPDF's `fmt_dict` order: /Type when its value is not null, then
    /// /Subtype if it is not null either, then the other entries as given.
    /// A reference printed as null counts as null, as MuPDF resolves it.
    fn compact_dictionary<'v>(
        &mut self,
        entries: impl IntoIterator<Item = (&'v [u8], &'v Object)>,
        depth: usize,
    ) -> Result<(), SerializeError> {
        let mut entries: Vec<(&[u8], &Object)> = entries.into_iter().collect();
        let present = |name: &[u8]| {
            entries.iter().any(|(key, value)| {
                *key == name
                    && !matches!(value, Object::Null)
                    && !matches!(value, Object::Reference(id) if self.nulls.contains(id))
            })
        };
        let typed = present(b"Type");
        let subtyped = typed && present(b"Subtype");
        if typed {
            entries.sort_by_key(|(key, _)| match *key {
                b"Type" => 0,
                b"Subtype" if subtyped => 1,
                _ => 2,
            });
        }
        self.separate = false;
        self.append(b"<<")?;
        for (key, value) in entries {
            self.object_charge()?;
            self.name(key)?;
            self.separate = true;
            self.compact(value, depth + 1)?;
            self.separate = true;
        }
        self.separate = false;
        self.append(b">>")
    }
    /// MuPDF's choice: uppercase hex for a string that starts with a UTF-16
    /// byte order mark or whose escapes would add more bytes than it has,
    /// otherwise a literal string.
    fn compact_string(&mut self, bytes: &[u8]) -> Result<(), SerializeError> {
        let extra = |byte: u8| match byte {
            b'\n' | b'\r' | b'\t' | 8 | 12 | b'(' | b')' | b'\\' => 1,
            0..=31 | 127..=255 => 3,
            _ => 0,
        };
        let added: usize = bytes.iter().map(|&b| extra(b)).sum();
        if bytes.starts_with(&[0xFE, 0xFF])
            || bytes.starts_with(&[0xFF, 0xFE])
            || added > bytes.len()
        {
            return self.hex_string(bytes);
        }
        let length = bytes
            .len()
            .checked_add(added)
            .and_then(|n| n.checked_add(2))
            .ok_or(SerializeError::OutputLimit)?;
        self.reserve(length)?;
        self.bytes.push(b'(');
        for &byte in bytes {
            match byte {
                b'\n' => self.bytes.extend_from_slice(b"\\n"),
                b'\r' => self.bytes.extend_from_slice(b"\\r"),
                b'\t' => self.bytes.extend_from_slice(b"\\t"),
                8 => self.bytes.extend_from_slice(b"\\b"),
                12 => self.bytes.extend_from_slice(b"\\f"),
                b'(' | b')' | b'\\' => self.bytes.extend_from_slice(&[b'\\', byte]),
                0..=31 | 127..=255 => self.bytes.extend_from_slice(&[
                    b'\\',
                    b'0' + (byte >> 6),
                    b'0' + ((byte >> 3) & 7),
                    b'0' + (byte & 7),
                ]),
                _ => self.bytes.push(byte),
            }
        }
        self.bytes.push(b')');
        Ok(())
    }
    fn hex_string(&mut self, bytes: &[u8]) -> Result<(), SerializeError> {
        let length = bytes
            .len()
            .checked_mul(2)
            .and_then(|n| n.checked_add(2))
            .ok_or(SerializeError::OutputLimit)?;
        self.reserve(length)?;
        self.bytes.push(b'<');
        for &byte in bytes {
            self.bytes
                .extend_from_slice(&[HEX[(byte >> 4) as usize], HEX[(byte & 15) as usize]]);
        }
        self.bytes.push(b'>');
        Ok(())
    }
    pub(crate) fn value(&mut self, value: &Object, depth: usize) -> Result<(), SerializeError> {
        if self.style == SerializeStyle::Compact {
            return self.compact(value, depth);
        }
        self.object_charge()?;
        match value {
            Object::Null => self.append(b"null"),
            Object::Boolean(true) => self.append(b"true"),
            Object::Boolean(false) => self.append(b"false"),
            Object::Integer(value) => self.append(value.to_string().as_bytes()),
            Object::Real(value) => {
                if !value.is_finite() {
                    return Err(SerializeError::NonfiniteReal);
                }
                let mut number = value.to_string();
                // Rust Display emits decimal notation; keep an explicit decimal
                // point so an integral real does not become a PDF integer.
                if !number.contains('.') {
                    number.push_str(".0");
                }
                self.append(number.as_bytes())
            }
            Object::Name(name) => self.name(name),
            Object::String(bytes) => self.hex_string(bytes),
            Object::Reference(id) if self.nulls.contains(id) => self.append(b"null"),
            Object::Reference(id) => {
                self.append(format!("{} {} R", id.number(), id.generation()).as_bytes())
            }
            Object::Array(values) => {
                if depth >= self.limits.max_depth.min(256) {
                    return Err(SerializeError::DepthLimit);
                }
                self.append(b"[")?;
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        self.append(b" ")?;
                    }
                    self.value(value, depth + 1)?;
                }
                self.append(b"]")
            }
            Object::Dictionary(values) => {
                if depth >= self.limits.max_depth.min(256) {
                    return Err(SerializeError::DepthLimit);
                }
                let entries = if self.keep_order {
                    values.iter().collect()
                } else {
                    values.sorted_entries()
                };
                self.append(b"<<")?;
                for (key, value) in entries {
                    self.object_charge()?;
                    self.append(b" ")?;
                    self.name(key)?;
                    self.append(b" ")?;
                    self.value(value, depth + 1)?;
                }
                self.append(b" >>")
            }
        }
    }
}
const HEX: &[u8; 16] = b"0123456789ABCDEF";
fn plain_name(byte: u8) -> bool {
    (b'!'..=b'~').contains(&byte) && !b"()<>[]{}/%#".contains(&byte)
}

/// MuPDF's real printing: the value rounded to f32; an integral value in
/// i32 range as that integer (-0 as 0); otherwise `fz_grisu`'s digits, the
/// shortest prefix of the upper rounding boundary's decimal expansion that
/// lies strictly above the lower boundary, with no exponent and no leading
/// zero before the point. MuPDF writes a value of 2^63 or more without a
/// point too; such a token gets a trailing point here so that it reads back
/// as a number.
pub(crate) fn compact_real(value: f64) -> String {
    let max = f64::from(f32::MAX);
    let single = value.clamp(-max, max) as f32;
    if single.fract() == 0.0 && (-2_147_483_648.0..2_147_483_648.0).contains(&single) {
        return (single as i32).to_string();
    }
    let (lower, upper) = boundaries(single.abs());
    let (digits, point) =
        shortest_inside(single.abs(), lower, upper).unwrap_or_else(|| below_upper(lower, upper));
    let mut out = String::new();
    if single < 0.0 {
        out.push('-');
    }
    if point <= 0 {
        out.push('.');
        out.extend(std::iter::repeat_n('0', point.unsigned_abs() as usize));
        out.extend(digits.iter().map(|&d| char::from(d)));
    } else {
        let point = point as usize;
        for (i, &d) in digits.iter().enumerate() {
            if i == point {
                out.push('.');
            }
            out.push(char::from(d));
        }
        out.extend(std::iter::repeat_n('0', point.saturating_sub(digits.len())));
        if digits.len() <= point && out.parse::<i64>().is_err() {
            out.push('.');
        }
    }
    out
}
/// The rounding boundaries of a positive f32: halfway to its neighbours.
/// Both need at most 26 significant bits, so f64 holds them exactly.
fn boundaries(single: f32) -> (f64, f64) {
    let bits = single.to_bits();
    let (m, e) = match bits >> 23 {
        0 => (bits, -149),
        biased => ((bits & 0x7F_FFFF) | 0x80_0000, biased as i32 - 150),
    };
    let scaled = |m: u32, e: i32| f64::from(m) * 2f64.powi(e);
    let lower = if m == 0x80_0000 {
        scaled(4 * m - 1, e - 2)
    } else {
        scaled(2 * m - 1, e - 1)
    };
    (lower, scaled(2 * m + 1, e - 1))
}
/// The fast path. Rust's shortest round-trip digits have the fewest digits
/// any number strictly between the boundaries has, when they lie strictly
/// between them; `below_upper`'s answer is then the largest number with as
/// many digits that is below the upper boundary. Correctly rounded parsing is
/// monotonic, so strict comparisons of parsed values are exact; anything
/// closer is left to `below_upper`.
fn shortest_inside(single: f32, lower: f64, upper: f64) -> Option<(Vec<u8>, i32)> {
    let text = format!("{single:e}");
    let (mantissa, exponent) = text.split_once('e')?;
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let point = exponent.parse::<i32>().ok()? + 1;
    let scale = point - i32::try_from(digits.len()).ok()?;
    let shortest: u64 = digits.parse().ok()?;
    let limit = 10u64.checked_pow(u32::try_from(digits.len()).ok()?)?;
    let parse = |n: u64| format!("{n}e{scale}").parse::<f64>().ok();
    let value = parse(shortest)?;
    if !(lower < value && value < upper) {
        return None;
    }
    let estimate = (upper / 10f64.powi(scale)).floor() as u64;
    for k in [
        estimate,
        estimate.saturating_add(1),
        estimate.saturating_sub(1),
    ] {
        if k < shortest || k >= limit {
            continue;
        }
        let below = k == shortest || parse(k)? < upper;
        if below && parse(k + 1)? > upper {
            let mut digits = k.to_string().into_bytes();
            trim(&mut digits);
            return Some((digits, point));
        }
    }
    None
}
/// The shortest prefix of the decimal expansion just below `upper` that is
/// greater than `lower`, as digits and point position.
fn below_upper(lower: f64, upper: f64) -> (Vec<u8>, i32) {
    let (mut upper, mut point) = digits(upper);
    let lower = digits(lower);
    // The digits of a value just below the upper boundary: the expansion
    // ends in a nonzero digit, which loses one unit and is followed by nines.
    if let Some(last) = upper.last_mut() {
        *last -= 1;
    }
    upper.extend([b'9'; 20]);
    if upper[0] == b'0' {
        upper.remove(0);
        point -= 1;
    }
    let mut prefix: &[u8] = &upper;
    for n in 1..=upper.len() {
        prefix = &upper[..n];
        while prefix.len() > 1 && prefix.last() == Some(&b'0') {
            prefix = &prefix[..prefix.len() - 1];
        }
        if greater((prefix, point), (&lower.0, lower.1)) {
            break;
        }
    }
    (prefix.to_vec(), point)
}
/// Exact decimal digits of a positive value without trailing zeros, and the
/// position of the decimal point: value = 0.DIGITS * 10^point.
fn digits(value: f64) -> (Vec<u8>, i32) {
    let text = format!("{value:.120e}");
    let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
    let mut digits: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    trim(&mut digits);
    (digits, exponent.parse::<i32>().unwrap_or(0) + 1)
}
fn trim(digits: &mut Vec<u8>) {
    while digits.len() > 1 && digits.last() == Some(&b'0') {
        digits.pop();
    }
}
/// Compare two positive values written as `digits` (no leading or trailing
/// zeros) with their decimal point position.
fn greater(a: (&[u8], i32), b: (&[u8], i32)) -> bool {
    match a.1.cmp(&b.1) {
        std::cmp::Ordering::Equal => a.0 > b.0,
        order => order == std::cmp::Ordering::Greater,
    }
}

/// Serialize into a new owned buffer. Failure returns no partial output.
/// Scalars require depth zero; containers use one level. Dictionary keys count
/// toward max_objects. The native recursion ceiling is always at most 256.
pub fn serialize_object(value: &Object, limits: WriteLimits) -> Result<Vec<u8>, SerializeError> {
    serialize_object_with_style(value, limits, SerializeStyle::Canonical)
}

/// `serialize_object` in the given style.
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
    use super::{below_upper, boundaries, shortest_inside};

    /// The fast path never changes a result: it agrees with the boundary
    /// search wherever it answers.
    #[test]
    fn fast_real_digits_agree_with_the_boundary_search() {
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut answered = [0usize; 2];
        let mut tried = [0usize; 2];
        for i in 0..120_000u32 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let single = if i % 2 == 0 {
                f32::from_bits(state as u32 & 0x7FFF_FFFF)
            } else {
                // Values with few decimals, as page coordinates have.
                (state % 2_000_000) as f32 / [10.0, 100.0, 1000.0][i as usize % 3]
            };
            if !single.is_finite() || single == 0.0 {
                continue;
            }
            let (lower, upper) = boundaries(single);
            tried[i as usize % 2] += 1;
            if let Some(fast) = shortest_inside(single, lower, upper) {
                assert_eq!(fast, below_upper(lower, upper), "{single:e}");
                answered[i as usize % 2] += 1;
            }
        }
        let edges = [
            f32::MIN_POSITIVE,
            f32::from_bits(1),
            f32::from_bits(0x007F_FFFF),
            f32::from_bits(0x0080_0001),
            f32::MAX,
            1.0,
            f32::EPSILON,
            0.1,
            1e-45,
            1.5e10,
            4_600_095_744.0,
        ];
        // Every power of two, denormal ones included.
        let powers = (0..23).map(|b| 1 << b).chain((1..255).map(|e| e << 23));
        for single in edges.into_iter().chain(powers.map(f32::from_bits)) {
            let (lower, upper) = boundaries(single);
            if let Some(fast) = shortest_inside(single, lower, upper) {
                assert_eq!(fast, below_upper(lower, upper), "{single:e}");
            }
        }
        // Nearly every value takes the fast path.
        assert!(answered[0] * 100 > tried[0] * 97, "{answered:?} {tried:?}");
        assert!(
            answered[1] * 1000 > tried[1] * 999,
            "{answered:?} {tried:?}"
        );
    }
}
