// SPDX-License-Identifier: AGPL-3.0-or-later
//! Owned, byte-preserving PDF object values. This module does not model streams.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt;
use std::iter::FusedIterator;
use std::sync::Arc;

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
/// keep their key order (see `Dictionary`); duplicate keys are handled by the parser's policy.
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

/// A PDF dictionary: keys in the order they were first inserted (ISO 32000-2
/// §7.3.7 gives dictionaries no order; keeping the source order keeps saved
/// files close to their input).
///
/// Storage is shared between clones until one side is written (copy on
/// write), so cloning is O(1). Small dictionaries are searched linearly;
/// larger ones keep a key index, and removal leaves a hole that is closed
/// later, so every operation stays O(log n) amortized.
#[derive(Clone)]
pub struct Dictionary {
    entries: Option<Arc<Entries>>,
}

/// One position of the entry list; `None` is a removed entry not yet
/// compacted away (only present while the key index exists).
type Slot = Option<(Vec<u8>, Object)>;

/// Above this many entries, lookups go through a key index.
const INDEXED_FROM: usize = 32;

#[derive(Clone, Default)]
struct Entries {
    slots: Vec<Slot>,
    live: usize,
    index: Option<BTreeMap<Vec<u8>, usize>>,
}

impl Entries {
    fn position<Q>(&self, key: &Q) -> Option<usize>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match &self.index {
            Some(index) => index.get(key).copied(),
            None => self
                .slots
                .iter()
                .position(|slot| matches!(slot, Some((k, _)) if k.borrow() == key)),
        }
    }

    fn slot(&self, position: usize) -> Option<&(Vec<u8>, Object)> {
        self.slots.get(position)?.as_ref()
    }

    fn insert(&mut self, key: Vec<u8>, value: Object) -> Option<Object> {
        if let Some(position) = self.position(key.as_slice()) {
            if let Some(Some((_, old))) = self.slots.get_mut(position) {
                return Some(std::mem::replace(old, value));
            }
        }
        let position = self.slots.len();
        if let Some(index) = &mut self.index {
            index.insert(key.clone(), position);
        }
        self.slots.push(Some((key, value)));
        self.live += 1;
        if self.index.is_none() && self.live > INDEXED_FROM {
            self.rebuild_index();
        }
        None
    }

    fn remove_at(&mut self, position: usize) -> Option<(Vec<u8>, Object)> {
        let removed = match &mut self.index {
            None => {
                if position < self.slots.len() {
                    self.slots.remove(position)
                } else {
                    None
                }
            }
            Some(index) => {
                let removed = self.slots.get_mut(position)?.take();
                if let Some((key, _)) = &removed {
                    index.remove(key);
                }
                removed
            }
        };
        if removed.is_some() {
            self.live -= 1;
            let holes = self.slots.len() - self.live;
            if holes > INDEXED_FROM && holes > self.live {
                self.compact();
            }
        }
        removed
    }

    /// Drops the holes and renumbers the index.
    fn compact(&mut self) {
        self.slots.retain(Option::is_some);
        self.live = self.slots.len();
        if self.index.is_some() {
            self.rebuild_index();
        }
    }

    fn rebuild_index(&mut self) {
        let index = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(position, slot)| slot.as_ref().map(|(key, _)| (key.clone(), position)))
            .collect();
        self.index = Some(index);
    }
}

impl Default for Dictionary {
    fn default() -> Self {
        Self::new()
    }
}

impl Dictionary {
    pub const fn new() -> Self {
        Self { entries: None }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        if capacity == 0 {
            return Self::new();
        }
        let mut entries = Entries::default();
        // A capacity is only a hint; failing to reserve it is not an error.
        let _ = entries.slots.try_reserve(capacity.min(1 << 16));
        Self {
            entries: Some(Arc::new(entries)),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.as_ref().map_or(0, |entries| entries.live)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&mut self) {
        self.entries = None;
    }

    fn position<Q>(&self, key: &Q) -> Option<usize>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.entries.as_ref()?.position(key)
    }

    /// The entries for writing, unshared first (copy on write).
    fn entries_mut(&mut self) -> &mut Entries {
        Arc::make_mut(self.entries.get_or_insert_with(Arc::default))
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.position(key).is_some()
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&Object>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.get_key_value(key).map(|(_, value)| value)
    }

    pub fn get_key_value<Q>(&self, key: &Q) -> Option<(&Vec<u8>, &Object)>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let entries = self.entries.as_ref()?;
        let (key, value) = entries.slot(entries.position(key)?)?;
        Some((key, value))
    }

    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut Object>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        // An absent key is not a write: the storage stays shared.
        let position = self.position(key)?;
        match self.entries_mut().slots.get_mut(position) {
            Some(Some((_, value))) => Some(value),
            _ => None,
        }
    }

    pub fn insert(&mut self, key: Vec<u8>, value: Object) -> Option<Object> {
        self.entries_mut().insert(key, value)
    }

    pub fn remove<Q>(&mut self, key: &Q) -> Option<Object>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.remove_entry(key).map(|(_, value)| value)
    }

    pub fn remove_entry<Q>(&mut self, key: &Q) -> Option<(Vec<u8>, Object)>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let position = self.position(key)?;
        self.entries_mut().remove_at(position)
    }

    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&Vec<u8>, &mut Object) -> bool,
    {
        if self.entries.is_none() {
            return;
        }
        let entries = self.entries_mut();
        for slot in &mut entries.slots {
            if let Some((key, value)) = slot {
                if !keep(key, value) {
                    *slot = None;
                }
            }
        }
        entries.compact();
        if entries.live <= INDEXED_FROM {
            entries.index = None;
        }
    }

    pub fn iter(&self) -> Iter<'_> {
        match &self.entries {
            Some(entries) => Iter {
                slots: entries.slots.iter(),
                remaining: entries.live,
            },
            None => Iter {
                slots: [].iter(),
                remaining: 0,
            },
        }
    }

    pub fn iter_mut(&mut self) -> IterMut<'_> {
        if self.entries.is_none() {
            return IterMut {
                slots: [].iter_mut(),
                remaining: 0,
            };
        }
        let entries = self.entries_mut();
        IterMut {
            remaining: entries.live,
            slots: entries.slots.iter_mut(),
        }
    }

    pub fn keys(&self) -> Keys<'_> {
        Keys { inner: self.iter() }
    }

    pub fn values(&self) -> Values<'_> {
        Values { inner: self.iter() }
    }

    pub fn values_mut(&mut self) -> ValuesMut<'_> {
        ValuesMut {
            inner: self.iter_mut(),
        }
    }

    pub fn into_keys(self) -> IntoKeys {
        IntoKeys {
            inner: self.into_iter(),
        }
    }

    pub fn into_values(self) -> IntoValues {
        IntoValues {
            inner: self.into_iter(),
        }
    }

    /// The entries in ascending byte order of their keys.
    pub fn sorted_entries(&self) -> Vec<(&Vec<u8>, &Object)> {
        let mut entries: Vec<_> = self.iter().collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
        entries
    }

    /// True when both share one unwritten, non-empty storage.
    pub fn shares_entries(&self, other: &Self) -> bool {
        match (&self.entries, &other.entries) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b) && a.live > 0,
            _ => false,
        }
    }
}

impl PartialEq for Dictionary {
    fn eq(&self, other: &Self) -> bool {
        if self.shares_entries(other) {
            return true;
        }
        self.len() == other.len()
            && self
                .iter()
                .all(|(key, value)| other.get(key.as_slice()) == Some(value))
    }
}

impl<Q> std::ops::Index<&Q> for Dictionary
where
    Vec<u8>: Borrow<Q>,
    Q: Ord + ?Sized,
{
    type Output = Object;
    fn index(&self, key: &Q) -> &Object {
        const NULL: &Object = &Object::Null;
        self.get(key).unwrap_or(NULL)
    }
}

impl fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl Extend<(Vec<u8>, Object)> for Dictionary {
    fn extend<I: IntoIterator<Item = (Vec<u8>, Object)>>(&mut self, entries: I) {
        for (key, value) in entries {
            self.insert(key, value);
        }
    }
}

impl FromIterator<(Vec<u8>, Object)> for Dictionary {
    fn from_iter<I: IntoIterator<Item = (Vec<u8>, Object)>>(entries: I) -> Self {
        let mut dictionary = Self::new();
        dictionary.extend(entries);
        dictionary
    }
}

impl<const N: usize> From<[(Vec<u8>, Object); N]> for Dictionary {
    fn from(entries: [(Vec<u8>, Object); N]) -> Self {
        entries.into_iter().collect()
    }
}

impl From<BTreeMap<Vec<u8>, Object>> for Dictionary {
    fn from(map: BTreeMap<Vec<u8>, Object>) -> Self {
        map.into_iter().collect()
    }
}

impl IntoIterator for Dictionary {
    type Item = (Vec<u8>, Object);
    type IntoIter = IntoIter;
    fn into_iter(self) -> IntoIter {
        let Some(entries) = self.entries else {
            return IntoIter {
                slots: Vec::new().into_iter(),
                remaining: 0,
            };
        };
        let entries = Arc::try_unwrap(entries).unwrap_or_else(|shared| (*shared).clone());
        IntoIter {
            remaining: entries.live,
            slots: entries.slots.into_iter(),
        }
    }
}

impl<'a> IntoIterator for &'a Dictionary {
    type Item = (&'a Vec<u8>, &'a Object);
    type IntoIter = Iter<'a>;
    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut Dictionary {
    type Item = (&'a Vec<u8>, &'a mut Object);
    type IntoIter = IterMut<'a>;
    fn into_iter(self) -> IterMut<'a> {
        self.iter_mut()
    }
}

/// Implements the iterator traits for an iterator over slots that skips
/// holes and counts the live entries it has left.
macro_rules! slot_iterator {
    ($name:ident $(<$lt:lifetime>)?, $item:ty, |$slot:ident| $map:expr) => {
        impl$(<$lt>)? Iterator for $name$(<$lt>)? {
            type Item = $item;
            fn next(&mut self) -> Option<$item> {
                for slot in self.slots.by_ref() {
                    if let Some($slot) = slot {
                        self.remaining -= 1;
                        return Some($map);
                    }
                }
                None
            }
            fn size_hint(&self) -> (usize, Option<usize>) {
                (self.remaining, Some(self.remaining))
            }
        }

        impl$(<$lt>)? DoubleEndedIterator for $name$(<$lt>)? {
            fn next_back(&mut self) -> Option<$item> {
                while let Some(slot) = self.slots.next_back() {
                    if let Some($slot) = slot {
                        self.remaining -= 1;
                        return Some($map);
                    }
                }
                None
            }
        }

        impl$(<$lt>)? ExactSizeIterator for $name$(<$lt>)? {}

        impl$(<$lt>)? FusedIterator for $name$(<$lt>)? {}
    };
}

/// Implements the iterator traits for an iterator that maps another one.
macro_rules! mapped_iterator {
    ($name:ident $(<$lt:lifetime>)?, $item:ty, |$entry:pat_param| $map:expr) => {
        impl$(<$lt>)? Iterator for $name$(<$lt>)? {
            type Item = $item;
            fn next(&mut self) -> Option<$item> {
                self.inner.next().map(|$entry| $map)
            }
            fn size_hint(&self) -> (usize, Option<usize>) {
                self.inner.size_hint()
            }
        }

        impl$(<$lt>)? DoubleEndedIterator for $name$(<$lt>)? {
            fn next_back(&mut self) -> Option<$item> {
                self.inner.next_back().map(|$entry| $map)
            }
        }

        impl$(<$lt>)? ExactSizeIterator for $name$(<$lt>)? {}

        impl$(<$lt>)? FusedIterator for $name$(<$lt>)? {}
    };
}

#[derive(Clone, Debug)]
pub struct Iter<'a> {
    slots: std::slice::Iter<'a, Slot>,
    remaining: usize,
}

slot_iterator!(Iter<'a>, (&'a Vec<u8>, &'a Object), |entry| (
    &entry.0, &entry.1
));

#[derive(Debug)]
pub struct IterMut<'a> {
    slots: std::slice::IterMut<'a, Slot>,
    remaining: usize,
}

slot_iterator!(IterMut<'a>, (&'a Vec<u8>, &'a mut Object), |entry| (
    &entry.0,
    &mut entry.1
));

#[derive(Clone, Debug)]
pub struct Keys<'a> {
    inner: Iter<'a>,
}

mapped_iterator!(Keys<'a>, &'a Vec<u8>, |(key, _)| key);

#[derive(Clone, Debug)]
pub struct Values<'a> {
    inner: Iter<'a>,
}

mapped_iterator!(Values<'a>, &'a Object, |(_, value)| value);

#[derive(Debug)]
pub struct ValuesMut<'a> {
    inner: IterMut<'a>,
}

mapped_iterator!(ValuesMut<'a>, &'a mut Object, |(_, value)| value);

#[derive(Debug)]
pub struct IntoIter {
    slots: std::vec::IntoIter<Slot>,
    remaining: usize,
}

slot_iterator!(IntoIter, (Vec<u8>, Object), |entry| entry);

#[derive(Debug)]
pub struct IntoKeys {
    inner: IntoIter,
}

mapped_iterator!(IntoKeys, Vec<u8>, |(key, _)| key);

#[derive(Debug)]
pub struct IntoValues {
    inner: IntoIter,
}

mapped_iterator!(IntoValues, Object, |(_, value)| value);

// Text strings (ISO 32000-2 §7.9.2.2) and PDFDocEncoding (Annex D).

/// The codes where PDFDocEncoding (ISO 32000-2 Annex D, Table D.2) differs
/// from ISO Latin-1, with the Unicode character each one stands for.
/// Written from the standard's table. Codes 0x7F, 0x9F and 0xAD are left
/// undefined by the table and are absent here.
const PDF_DOC_SPECIAL: [(u8, char); 40] = [
    (0x18, '\u{02D8}'), // breve
    (0x19, '\u{02C7}'), // caron
    (0x1A, '\u{02C6}'), // circumflex
    (0x1B, '\u{02D9}'), // dot above
    (0x1C, '\u{02DD}'), // double acute
    (0x1D, '\u{02DB}'), // ogonek
    (0x1E, '\u{02DA}'), // ring above
    (0x1F, '\u{02DC}'), // small tilde
    (0x80, '\u{2022}'), // bullet
    (0x81, '\u{2020}'), // dagger
    (0x82, '\u{2021}'), // double dagger
    (0x83, '\u{2026}'), // ellipsis
    (0x84, '\u{2014}'), // em dash
    (0x85, '\u{2013}'), // en dash
    (0x86, '\u{0192}'), // florin
    (0x87, '\u{2044}'), // fraction slash
    (0x88, '\u{2039}'), // single left angle quotation mark
    (0x89, '\u{203A}'), // single right angle quotation mark
    (0x8A, '\u{2212}'), // minus
    (0x8B, '\u{2030}'), // per mille
    (0x8C, '\u{201E}'), // double low-9 quotation mark
    (0x8D, '\u{201C}'), // left double quotation mark
    (0x8E, '\u{201D}'), // right double quotation mark
    (0x8F, '\u{2018}'), // left single quotation mark
    (0x90, '\u{2019}'), // right single quotation mark
    (0x91, '\u{201A}'), // single low-9 quotation mark
    (0x92, '\u{2122}'), // trade mark
    (0x93, '\u{FB01}'), // fi ligature
    (0x94, '\u{FB02}'), // fl ligature
    (0x95, '\u{0141}'), // L with stroke
    (0x96, '\u{0152}'), // OE ligature
    (0x97, '\u{0160}'), // S with caron
    (0x98, '\u{0178}'), // Y with diaeresis
    (0x99, '\u{017D}'), // Z with caron
    (0x9A, '\u{0131}'), // dotless i
    (0x9B, '\u{0142}'), // l with stroke
    (0x9C, '\u{0153}'), // oe ligature
    (0x9D, '\u{0161}'), // s with caron
    (0x9E, '\u{017E}'), // z with caron
    (0xA0, '\u{20AC}'), // euro sign
];

/// The character a PDFDocEncoding byte stands for. Undefined codes and the
/// control codes read as the code point with the same number.
fn pdf_doc_char(byte: u8) -> char {
    PDF_DOC_SPECIAL
        .iter()
        .find(|(code, _)| *code == byte)
        .map_or(char::from(byte), |(_, c)| *c)
}

pub(crate) fn pdf_doc_byte(c: char) -> Option<u8> {
    if let Some((code, _)) = PDF_DOC_SPECIAL.iter().find(|(_, mapped)| *mapped == c) {
        return Some(*code);
    }
    match u32::from(c) {
        value @ (0x00..=0x17 | 0x20..=0x7E | 0xA1..=0xAC | 0xAE..=0xFF) => u8::try_from(value).ok(),
        _ => None,
    }
}

/// Bytes PDFDocEncoding reads as accents rather than as their ASCII controls.
fn is_accent_code(byte: u8) -> bool {
    (0x18..=0x1F).contains(&byte)
}

pub fn encode_text_string(text: &str) -> Vec<u8> {
    let plain = text
        .chars()
        .all(|c| c.is_ascii() && !is_accent_code(c as u8));
    if plain {
        return text.as_bytes().to_vec();
    }
    let mut out = Vec::with_capacity(2 + 2 * text.len());
    out.extend_from_slice(&[0xFE, 0xFF]);
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

/// How a text string's bytes are encoded (ISO 32000-2 §7.9.2.2).
#[derive(Clone, Copy)]
enum TextEncoding {
    Utf16 {
        big_endian: bool,
    },
    /// UTF-8 after `skip` mark bytes.
    Utf8 {
        skip: usize,
    },
    PdfDoc,
}

fn text_encoding(bytes: &[u8]) -> TextEncoding {
    match bytes {
        [0xFE, 0xFF, ..] => TextEncoding::Utf16 { big_endian: true },
        [0xFF, 0xFE, ..] => TextEncoding::Utf16 { big_endian: false },
        [0xEF, 0xBB, 0xBF, ..] => TextEncoding::Utf8 { skip: 3 },
        _ if bytes.iter().any(|&b| b >= 0x80)
            && !bytes.iter().any(|&b| is_accent_code(b))
            && std::str::from_utf8(bytes).is_ok() =>
        {
            TextEncoding::Utf8 { skip: 0 }
        }
        _ => TextEncoding::PdfDoc,
    }
}

/// Decodes `bytes` (mark included) as `encoding`. With `cut`, a character
/// the end of `bytes` splits is left out instead of becoming U+FFFD.
fn decode_with(bytes: &[u8], encoding: TextEncoding, cut: bool) -> String {
    match encoding {
        TextEncoding::Utf16 { big_endian } => {
            let body = bytes.get(2..).unwrap_or_default();
            let mut units: Vec<u16> = body
                .chunks_exact(2)
                .map(|pair| {
                    let pair = [pair[0], pair[1]];
                    if big_endian {
                        u16::from_be_bytes(pair)
                    } else {
                        u16::from_le_bytes(pair)
                    }
                })
                .collect();
            if cut
                && units
                    .last()
                    .is_some_and(|unit| (0xD800..0xDC00).contains(unit))
            {
                units.pop();
            }
            char::decode_utf16(units)
                .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect()
        }
        TextEncoding::Utf8 { skip } => {
            let mut rest = bytes.get(skip..).unwrap_or_default();
            let mut out = String::with_capacity(rest.len());
            loop {
                match std::str::from_utf8(rest) {
                    Ok(valid) => {
                        out.push_str(valid);
                        break;
                    }
                    Err(error) => {
                        let (valid, after) = rest.split_at(error.valid_up_to());
                        out.push_str(std::str::from_utf8(valid).unwrap_or_default());
                        match error.error_len() {
                            Some(length) => {
                                out.push(char::REPLACEMENT_CHARACTER);
                                rest = after.get(length..).unwrap_or_default();
                            }
                            // An incomplete sequence at the end.
                            None => {
                                if !cut {
                                    out.push(char::REPLACEMENT_CHARACTER);
                                }
                                break;
                            }
                        }
                    }
                }
            }
            out
        }
        TextEncoding::PdfDoc => bytes.iter().map(|&b| pdf_doc_char(b)).collect(),
    }
}

pub fn decode_text_string(bytes: &[u8]) -> String {
    decode_with(bytes, text_encoding(bytes), false)
}

pub fn decode_text_prefix(bytes: &[u8], max: usize) -> (String, bool) {
    let encoding = text_encoding(bytes);
    if bytes.len() <= max {
        return (decode_with(bytes, encoding, false), false);
    }
    let prefix = bytes.get(..max).unwrap_or(bytes);
    (decode_with(prefix, encoding, true), true)
}
