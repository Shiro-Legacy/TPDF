// SPDX-License-Identifier: AGPL-3.0-or-later
//! Owned, byte-preserving PDF object values. This module does not model streams.

use std::collections::BTreeMap;
use std::fmt;

/// A valid indirect-reference target. Object zero is the free-list sentinel and
/// is rejected by this strict object parser. Generation zero is valid.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectId {
    number: u32,
    generation: u16,
}

impl ObjectId {
    pub fn new(number: u32, generation: u16) -> Result<Self, InvalidObjectId> {
        if number == 0 {
            return Err(InvalidObjectId);
        }
        Ok(Self { number, generation })
    }

    pub const fn number(self) -> u32 {
        self.number
    }

    pub const fn generation(self) -> u16 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidObjectId;

impl fmt::Display for InvalidObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("indirect object number must be greater than zero")
    }
}

impl std::error::Error for InvalidObjectId {}

/// PDF strings and names are bytes, not implicitly decoded Unicode. Dictionary
/// ordering is deterministic; duplicate keys are handled by the parser's policy.
#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f64),
    Name(Vec<u8>),
    String(Vec<u8>),
    Array(Vec<Object>),
    Dictionary(BTreeMap<Vec<u8>, Object>),
    Reference(ObjectId),
}
