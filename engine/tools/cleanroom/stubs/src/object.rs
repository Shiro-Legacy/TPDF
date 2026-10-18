// SPDX-License-Identifier: AGPL-3.0-or-later
//! Owned, byte-preserving PDF object values. This module does not model streams.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt;
use std::iter::FusedIterator;
use std::marker::PhantomData;

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

/// PDF strings and names are bytes, not implicitly decoded Unicode. Dictionaries
/// keep MuPDF's key order; duplicate keys are handled by the parser's policy.
#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Null,
    Boolean(bool),
    Integer(i64),
    Real(f64),
    Name(Vec<u8>),
    String(Vec<u8>),
    Array(Vec<Object>),
    Dictionary(Dictionary),
    Reference(ObjectId),
}

impl Object {
    pub fn name(name: impl Into<Vec<u8>>) -> Self {
        Self::Name(name.into())
    }
    /// A byte string, stored as given.
    pub fn string(bytes: impl Into<Vec<u8>>) -> Self {
        Self::String(bytes.into())
    }
    /// A text string encoded by `encode_text_string`.
    pub fn text(text: &str) -> Self {
        Self::String(encode_text_string(text))
    }
    pub fn reference(id: ObjectId) -> Self {
        Self::Reference(id)
    }
    pub fn integer(value: i64) -> Self {
        Self::Integer(value)
    }
    /// `None` for NaN and infinities, which PDF cannot represent.
    pub fn real(value: f64) -> Option<Self> {
        value.is_finite().then_some(Self::Real(value))
    }
    pub fn dictionary<K: Into<Vec<u8>>>(entries: impl IntoIterator<Item = (K, Object)>) -> Self {
        Self::Dictionary(entries.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
    pub fn array(items: impl IntoIterator<Item = Object>) -> Self {
        Self::Array(items.into_iter().collect())
    }

    pub fn as_dict(&self) -> Option<&Dictionary> {
        match self {
            Self::Dictionary(d) => Some(d),
            _ => None,
        }
    }
    pub fn as_dict_mut(&mut self) -> Option<&mut Dictionary> {
        match self {
            Self::Dictionary(d) => Some(d),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Object]> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Object>> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Self::Name(n) => Some(n),
            _ => None,
        }
    }
    pub fn as_string(&self) -> Option<&[u8]> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(i) => Some(*i),
            _ => None,
        }
    }
    /// Integers and reals both read as numbers.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Integer(i) => Some(*i as f64),
            Self::Real(r) => Some(*r),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_reference(&self) -> Option<ObjectId> {
        match self {
            Self::Reference(id) => Some(*id),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

#[derive(Clone)]
pub struct Dictionary {
    _entries: (),
}

impl Default for Dictionary {
    fn default() -> Self {
        todo!("clean-room: spec 01")
    }
}

impl Dictionary {
    pub const fn new() -> Self {
        Self { _entries: () }
    }

    pub fn with_capacity(_capacity: usize) -> Self {
        todo!("clean-room: spec 01")
    }

    pub fn len(&self) -> usize {
        todo!("clean-room: spec 01")
    }

    pub fn is_empty(&self) -> bool {
        todo!("clean-room: spec 01")
    }

    pub fn clear(&mut self) {
        todo!("clean-room: spec 01")
    }

    pub fn contains_key<Q>(&self, _key: &Q) -> bool
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        todo!("clean-room: spec 01")
    }

    pub fn get<Q>(&self, _key: &Q) -> Option<&Object>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        todo!("clean-room: spec 01")
    }

    pub fn get_key_value<Q>(&self, _key: &Q) -> Option<(&Vec<u8>, &Object)>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        todo!("clean-room: spec 01")
    }

    pub fn get_mut<Q>(&mut self, _key: &Q) -> Option<&mut Object>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        todo!("clean-room: spec 01")
    }

    pub fn insert(&mut self, _key: Vec<u8>, _value: Object) -> Option<Object> {
        todo!("clean-room: spec 01")
    }

    pub fn remove<Q>(&mut self, _key: &Q) -> Option<Object>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        todo!("clean-room: spec 01")
    }

    pub fn remove_entry<Q>(&mut self, _key: &Q) -> Option<(Vec<u8>, Object)>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        todo!("clean-room: spec 01")
    }

    pub fn retain<F>(&mut self, _keep: F)
    where
        F: FnMut(&Vec<u8>, &mut Object) -> bool,
    {
        todo!("clean-room: spec 01")
    }

    pub fn iter(&self) -> Iter<'_> {
        todo!("clean-room: spec 01")
    }

    pub fn iter_mut(&mut self) -> IterMut<'_> {
        todo!("clean-room: spec 01")
    }

    pub fn keys(&self) -> Keys<'_> {
        todo!("clean-room: spec 01")
    }

    pub fn values(&self) -> Values<'_> {
        todo!("clean-room: spec 01")
    }

    pub fn values_mut(&mut self) -> ValuesMut<'_> {
        todo!("clean-room: spec 01")
    }

    pub fn into_keys(self) -> IntoKeys {
        todo!("clean-room: spec 01")
    }

    pub fn into_values(self) -> IntoValues {
        todo!("clean-room: spec 01")
    }

    pub fn is_sorted(&self) -> bool {
        todo!("clean-room: spec 01")
    }

    pub fn sorted_entries(&self) -> Vec<(&Vec<u8>, &Object)> {
        todo!("clean-room: spec 01")
    }

    pub fn shares_entries(&self, _other: &Self) -> bool {
        todo!("clean-room: spec 01")
    }
}

impl PartialEq for Dictionary {
    fn eq(&self, _other: &Self) -> bool {
        todo!("clean-room: spec 01")
    }
}

impl<Q> std::ops::Index<&Q> for Dictionary
where
    Vec<u8>: Borrow<Q>,
    Q: Ord + ?Sized,
{
    type Output = Object;
    fn index(&self, _key: &Q) -> &Object {
        todo!("clean-room: spec 01")
    }
}

impl fmt::Debug for Dictionary {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!("clean-room: spec 01")
    }
}

impl Extend<(Vec<u8>, Object)> for Dictionary {
    fn extend<I: IntoIterator<Item = (Vec<u8>, Object)>>(&mut self, _entries: I) {
        todo!("clean-room: spec 01")
    }
}

impl FromIterator<(Vec<u8>, Object)> for Dictionary {
    fn from_iter<I: IntoIterator<Item = (Vec<u8>, Object)>>(_entries: I) -> Self {
        todo!("clean-room: spec 01")
    }
}

impl<const N: usize> From<[(Vec<u8>, Object); N]> for Dictionary {
    fn from(_entries: [(Vec<u8>, Object); N]) -> Self {
        todo!("clean-room: spec 01")
    }
}

impl From<BTreeMap<Vec<u8>, Object>> for Dictionary {
    fn from(_map: BTreeMap<Vec<u8>, Object>) -> Self {
        todo!("clean-room: spec 01")
    }
}

impl IntoIterator for Dictionary {
    type Item = (Vec<u8>, Object);
    type IntoIter = IntoIter;
    fn into_iter(self) -> IntoIter {
        todo!("clean-room: spec 01")
    }
}

impl<'a> IntoIterator for &'a Dictionary {
    type Item = (&'a Vec<u8>, &'a Object);
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Iter<'a> {
        todo!("clean-room: spec 01")
    }
}

impl<'a> IntoIterator for &'a mut Dictionary {
    type Item = (&'a Vec<u8>, &'a mut Object);
    type IntoIter = IterMut<'a>;
    fn into_iter(self) -> IterMut<'a> {
        todo!("clean-room: spec 01")
    }
}

#[derive(Clone, Debug)]
pub struct Iter<'a> {
    _entries: PhantomData<&'a ()>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = (&'a Vec<u8>, &'a Object);
    fn next(&mut self) -> Option<(&'a Vec<u8>, &'a Object)> {
        todo!("clean-room: spec 01")
    }
}

impl<'a> DoubleEndedIterator for Iter<'a> {
    fn next_back(&mut self) -> Option<(&'a Vec<u8>, &'a Object)> {
        todo!("clean-room: spec 01")
    }
}

impl<'a> ExactSizeIterator for Iter<'a> {}

impl<'a> FusedIterator for Iter<'a> {}

#[derive(Debug)]
pub struct IterMut<'a> {
    _entries: PhantomData<&'a ()>,
}

impl<'a> Iterator for IterMut<'a> {
    type Item = (&'a Vec<u8>, &'a mut Object);
    fn next(&mut self) -> Option<(&'a Vec<u8>, &'a mut Object)> {
        todo!("clean-room: spec 01")
    }
}

impl<'a> DoubleEndedIterator for IterMut<'a> {
    fn next_back(&mut self) -> Option<(&'a Vec<u8>, &'a mut Object)> {
        todo!("clean-room: spec 01")
    }
}

impl<'a> ExactSizeIterator for IterMut<'a> {}

impl<'a> FusedIterator for IterMut<'a> {}

#[derive(Clone, Debug)]
pub struct Keys<'a> {
    _entries: PhantomData<&'a ()>,
}

impl<'a> Iterator for Keys<'a> {
    type Item = &'a Vec<u8>;
    fn next(&mut self) -> Option<&'a Vec<u8>> {
        todo!("clean-room: spec 01")
    }
}

impl<'a> DoubleEndedIterator for Keys<'a> {
    fn next_back(&mut self) -> Option<&'a Vec<u8>> {
        todo!("clean-room: spec 01")
    }
}

impl<'a> ExactSizeIterator for Keys<'a> {}

impl<'a> FusedIterator for Keys<'a> {}

#[derive(Clone, Debug)]
pub struct Values<'a> {
    _entries: PhantomData<&'a ()>,
}

impl<'a> Iterator for Values<'a> {
    type Item = &'a Object;
    fn next(&mut self) -> Option<&'a Object> {
        todo!("clean-room: spec 01")
    }
}

impl<'a> DoubleEndedIterator for Values<'a> {
    fn next_back(&mut self) -> Option<&'a Object> {
        todo!("clean-room: spec 01")
    }
}

impl<'a> ExactSizeIterator for Values<'a> {}

impl<'a> FusedIterator for Values<'a> {}

#[derive(Debug)]
pub struct ValuesMut<'a> {
    _entries: PhantomData<&'a ()>,
}

impl<'a> Iterator for ValuesMut<'a> {
    type Item = &'a mut Object;
    fn next(&mut self) -> Option<&'a mut Object> {
        todo!("clean-room: spec 01")
    }
}

impl<'a> DoubleEndedIterator for ValuesMut<'a> {
    fn next_back(&mut self) -> Option<&'a mut Object> {
        todo!("clean-room: spec 01")
    }
}

impl<'a> ExactSizeIterator for ValuesMut<'a> {}

impl<'a> FusedIterator for ValuesMut<'a> {}

#[derive(Debug)]
pub struct IntoIter {
    _entries: PhantomData<()>,
}

impl Iterator for IntoIter {
    type Item = (Vec<u8>, Object);
    fn next(&mut self) -> Option<(Vec<u8>, Object)> {
        todo!("clean-room: spec 01")
    }
}

impl DoubleEndedIterator for IntoIter {
    fn next_back(&mut self) -> Option<(Vec<u8>, Object)> {
        todo!("clean-room: spec 01")
    }
}

impl ExactSizeIterator for IntoIter {}

impl FusedIterator for IntoIter {}

#[derive(Debug)]
pub struct IntoKeys {
    _entries: PhantomData<()>,
}

impl Iterator for IntoKeys {
    type Item = Vec<u8>;
    fn next(&mut self) -> Option<Vec<u8>> {
        todo!("clean-room: spec 01")
    }
}

impl DoubleEndedIterator for IntoKeys {
    fn next_back(&mut self) -> Option<Vec<u8>> {
        todo!("clean-room: spec 01")
    }
}

impl ExactSizeIterator for IntoKeys {}

impl FusedIterator for IntoKeys {}

#[derive(Debug)]
pub struct IntoValues {
    _entries: PhantomData<()>,
}

impl Iterator for IntoValues {
    type Item = Object;
    fn next(&mut self) -> Option<Object> {
        todo!("clean-room: spec 01")
    }
}

impl DoubleEndedIterator for IntoValues {
    fn next_back(&mut self) -> Option<Object> {
        todo!("clean-room: spec 01")
    }
}

impl ExactSizeIterator for IntoValues {}

impl FusedIterator for IntoValues {}

pub(crate) fn pdf_doc_byte(_c: char) -> Option<u8> {
    todo!("clean-room: spec 01")
}

pub fn encode_text_string(_text: &str) -> Vec<u8> {
    todo!("clean-room: spec 01")
}

pub fn decode_text_string(_bytes: &[u8]) -> String {
    todo!("clean-room: spec 01")
}

pub fn decode_text_prefix(_bytes: &[u8], _max: usize) -> (String, bool) {
    todo!("clean-room: spec 01")
}
