// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded canonical serialization of owned PDF objects, not whole documents.
use crate::object::Object;
use std::fmt;

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
}
impl Writer {
    pub(crate) fn new(limits: WriteLimits) -> Self {
        Self {
            bytes: Vec::new(),
            limits,
            objects: 0,
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }
    pub(crate) fn finish(self) -> Vec<u8> {
        self.bytes
    }
    /// Emit a stream dictionary without cloning any values or decoded payloads.
    pub(crate) fn stream_dictionary(
        &mut self,
        values: &std::collections::BTreeMap<Vec<u8>, Object>,
        length: usize,
    ) -> Result<(), SerializeError> {
        self.object_charge()?;
        if self.limits.max_depth == 0 {
            return Err(SerializeError::DepthLimit);
        }
        let length = i64::try_from(length).map_err(|_| SerializeError::OutputLimit)?;
        self.append(b"<<")?;
        let mut emitted = false;
        for (key, value) in values {
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
    pub(crate) fn value(&mut self, value: &Object, depth: usize) -> Result<(), SerializeError> {
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
            Object::String(bytes) => {
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
                self.append(b"<<")?;
                for (key, value) in values {
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

/// Serialize into a new owned buffer. Failure returns no partial output.
/// Scalars require depth zero; containers use one level. Dictionary keys count
/// toward max_objects. The native recursion ceiling is always at most 256.
pub fn serialize_object(value: &Object, limits: WriteLimits) -> Result<Vec<u8>, SerializeError> {
    let mut writer = Writer::new(limits);
    writer.value(value, 0)?;
    Ok(writer.finish())
}
