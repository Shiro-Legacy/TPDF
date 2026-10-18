// SPDX-License-Identifier: AGPL-3.0-or-later
//! Owned, byte-preserving PDF object values. This module does not model streams.

use std::borrow::Borrow;
use std::collections::{BTreeMap, btree_map};
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

/// MuPDF sorts a dictionary at the first put that finds more than this many
/// entries (`pdf_dict_get_put`).
pub(crate) const SORT_AFTER: usize = 100;

/// A PDF dictionary ordered as MuPDF orders its dictionaries on insertion, so
/// a saved dictionary keeps its source key order and a new one the order its
/// keys were added in. Replacing a value keeps the key's position. As in
/// MuPDF, an insertion (new key or replaced value) into a dictionary that
/// already holds more than 100 entries sorts it by key bytes. Unlike MuPDF,
/// whose `pdf_dict_del` moves the last entry into the hole and unsorts the
/// dictionary, removal keeps the order of the others and a sorted dictionary
/// stays sorted. Writes through `get_mut`, `iter_mut` or `values_mut` never
/// sort; use `insert` where MuPDF's put order matters.
///
/// An insertion-ordered dictionary therefore holds at most 101 entries and is
/// searched linearly; a sorted one is a `BTreeMap`, so lookup and insertion
/// are O(log n) at any size. Equality ignores order, as PDF does:
/// dictionaries are equal when they hold equal values under the same keys.
///
/// Clones share their entries until one of them is written (copy on
/// write): cloning is O(1), and a write copies only the dictionaries on its
/// path, each shallowly, so a journal state and the live value share every
/// dictionary the edit did not reach.
#[derive(Clone)]
pub struct Dictionary {
    entries: Entries,
}

#[derive(Clone)]
enum Entries {
    /// `None` until the first entry, so an empty dictionary allocates nothing.
    Ordered(Option<Arc<Vec<Slot>>>),
    Sorted(Arc<BTreeMap<Vec<u8>, Object>>),
}

type Slot = (Vec<u8>, Object);

/// The entries for reading.
enum View<'a> {
    Ordered(&'a [Slot]),
    Sorted(&'a BTreeMap<Vec<u8>, Object>),
}

/// The entries for writing in place, unshared first.
enum ViewMut<'a> {
    Ordered(&'a mut [Slot]),
    Sorted(&'a mut BTreeMap<Vec<u8>, Object>),
}

impl Entries {
    fn view(&self) -> View<'_> {
        match self {
            Self::Ordered(entries) => View::Ordered(entries.as_deref().map_or(&[], Vec::as_slice)),
            Self::Sorted(map) => View::Sorted(map),
        }
    }
    fn view_mut(&mut self) -> ViewMut<'_> {
        match self {
            Self::Ordered(None) => ViewMut::Ordered(&mut []),
            Self::Ordered(Some(entries)) => ViewMut::Ordered(Arc::make_mut(entries).as_mut_slice()),
            Self::Sorted(map) => ViewMut::Sorted(Arc::make_mut(map)),
        }
    }
    /// Takes the entries out, unshared, leaving the dictionary empty.
    fn take(&mut self) -> Either<Vec<Slot>, BTreeMap<Vec<u8>, Object>> {
        match std::mem::replace(self, Self::Ordered(None)) {
            Self::Ordered(entries) => {
                Either::Ordered(entries.map(Arc::unwrap_or_clone).unwrap_or_default())
            }
            Self::Sorted(map) => Either::Sorted(Arc::unwrap_or_clone(map)),
        }
    }
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ordered(Some(a)), Self::Ordered(Some(b))) => Arc::ptr_eq(a, b),
            (Self::Sorted(a), Self::Sorted(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Default for Dictionary {
    fn default() -> Self {
        Self::new()
    }
}

impl Dictionary {
    pub const fn new() -> Self {
        Self {
            entries: Entries::Ordered(None),
        }
    }
    pub fn with_capacity(capacity: usize) -> Self {
        let entries =
            (capacity > 0).then(|| Arc::new(Vec::with_capacity(capacity.min(SORT_AFTER + 1))));
        Self {
            entries: Entries::Ordered(entries),
        }
    }
    pub fn len(&self) -> usize {
        match self.entries.view() {
            View::Ordered(entries) => entries.len(),
            View::Sorted(map) => map.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Remove every entry; a sorted dictionary stays sorted.
    pub fn clear(&mut self) {
        self.entries = match self.entries {
            Entries::Ordered(_) => Entries::Ordered(None),
            Entries::Sorted(_) => Entries::Sorted(Arc::default()),
        };
    }

    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.get_key_value(key).is_some()
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
        match self.entries.view() {
            View::Ordered(entries) => entries
                .iter()
                .find(|(k, _)| k.borrow() == key)
                .map(|(k, v)| (k, v)),
            View::Sorted(map) => map.get_key_value(key),
        }
    }
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut Object>
    where
        Vec<u8>: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        // An absent key unshares nothing.
        if !self.contains_key(key) {
            return None;
        }
        match self.entries.view_mut() {
            ViewMut::Ordered(entries) => entries
                .iter_mut()
                .find(|(k, _)| (*k).borrow() == key)
                .map(|(_, v)| v),
            ViewMut::Sorted(map) => map.get_mut(key),
        }
    }
    /// Set a value and return the one it replaces. A key already present
    /// keeps its position; a new key goes last, or in key order once the
    /// dictionary is sorted.
    pub fn insert(&mut self, key: Vec<u8>, value: Object) -> Option<Object> {
        match &mut self.entries {
            Entries::Sorted(map) => Arc::make_mut(map).insert(key, value),
            Entries::Ordered(Some(entries)) if entries.len() > SORT_AFTER => {
                let mut map: BTreeMap<_, _> = match self.entries.take() {
                    Either::Ordered(entries) => entries.into_iter().collect(),
                    Either::Sorted(map) => map,
                };
                let old = map.insert(key, value);
                self.entries = Entries::Sorted(Arc::new(map));
                old
            }
            Entries::Ordered(entries) => {
                let entries = Arc::make_mut(entries.get_or_insert_with(Arc::default));
                match entries.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, old)) => Some(std::mem::replace(old, value)),
                    None => {
                        entries.push((key, value));
                        None
                    }
                }
            }
        }
    }
    /// Remove a key; the others keep their order, sorted or not.
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
        if !self.contains_key(key) {
            return None;
        }
        match &mut self.entries {
            Entries::Ordered(None) => None,
            Entries::Ordered(Some(entries)) => {
                let entries = Arc::make_mut(entries);
                let at = entries.iter().position(|(k, _)| k.borrow() == key)?;
                Some(entries.remove(at))
            }
            Entries::Sorted(map) => Arc::make_mut(map).remove_entry(key),
        }
    }
    /// Keep the entries for which `keep` returns true, in order.
    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(&Vec<u8>, &mut Object) -> bool,
    {
        match &mut self.entries {
            Entries::Ordered(None) => {}
            Entries::Ordered(Some(entries)) => {
                Arc::make_mut(entries).retain_mut(|(key, value)| keep(key, value))
            }
            Entries::Sorted(map) => Arc::make_mut(map).retain(|key, value| keep(key, value)),
        }
    }

    pub fn iter(&self) -> Iter<'_> {
        Iter(match self.entries.view() {
            View::Ordered(entries) => Either::Ordered(entries.iter().map(pair as _)),
            View::Sorted(map) => Either::Sorted(map.iter()),
        })
    }
    pub fn iter_mut(&mut self) -> IterMut<'_> {
        IterMut(match self.entries.view_mut() {
            ViewMut::Ordered(entries) => Either::Ordered(entries.iter_mut().map(pair_mut as _)),
            ViewMut::Sorted(map) => Either::Sorted(map.iter_mut()),
        })
    }
    pub fn keys(&self) -> Keys<'_> {
        Keys(match self.entries.view() {
            View::Ordered(entries) => Either::Ordered(entries.iter().map(key as _)),
            View::Sorted(map) => Either::Sorted(map.keys()),
        })
    }
    pub fn values(&self) -> Values<'_> {
        Values(match self.entries.view() {
            View::Ordered(entries) => Either::Ordered(entries.iter().map(value as _)),
            View::Sorted(map) => Either::Sorted(map.values()),
        })
    }
    pub fn values_mut(&mut self) -> ValuesMut<'_> {
        ValuesMut(match self.entries.view_mut() {
            ViewMut::Ordered(entries) => Either::Ordered(entries.iter_mut().map(value_mut as _)),
            ViewMut::Sorted(map) => Either::Sorted(map.values_mut()),
        })
    }
    pub fn into_keys(mut self) -> IntoKeys {
        IntoKeys(match self.entries.take() {
            Either::Ordered(entries) => Either::Ordered(entries.into_iter().map(owned_key as _)),
            Either::Sorted(map) => Either::Sorted(map.into_keys()),
        })
    }
    pub fn into_values(mut self) -> IntoValues {
        IntoValues(match self.entries.take() {
            Either::Ordered(entries) => Either::Ordered(entries.into_iter().map(owned_value as _)),
            Either::Sorted(map) => Either::Sorted(map.into_values()),
        })
    }
    /// Whether the dictionary is in key order for good, as MuPDF sorts a
    /// dictionary of more than 100 entries.
    pub fn is_sorted(&self) -> bool {
        matches!(self.entries, Entries::Sorted(_))
    }
    /// The entries in byte order of their keys.
    pub fn sorted_entries(&self) -> Vec<(&Vec<u8>, &Object)> {
        let mut entries: Vec<_> = self.iter().collect();
        if !self.is_sorted() {
            entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
        }
        entries
    }
    /// Whether `other` shares this dictionary's entries (a clone neither
    /// has written since): then they are equal without comparing.
    pub fn shares_entries(&self, other: &Self) -> bool {
        self.entries.same(&other.entries)
    }
}

impl PartialEq for Dictionary {
    fn eq(&self, other: &Self) -> bool {
        if self.shares_entries(other) {
            return true;
        }
        if self.len() != other.len() {
            return false;
        }
        match (self.entries.view(), other.entries.view()) {
            (View::Sorted(a), View::Sorted(b)) => a == b,
            (View::Ordered(a), View::Sorted(b)) | (View::Sorted(b), View::Ordered(a)) => {
                a.iter().all(|(key, value)| b.get(key) == Some(value))
            }
            // The same keys in the same order, the usual case, in one pass;
            // otherwise both in key order (at most 101 entries each).
            (View::Ordered(a), View::Ordered(b)) => {
                if a.iter().zip(b).all(|(x, y)| x.0 == y.0) {
                    return a.iter().zip(b).all(|(x, y)| x.1 == y.1);
                }
                fn by_key(entries: &[Slot]) -> Vec<&Slot> {
                    let mut sorted: Vec<&Slot> = entries.iter().collect();
                    sorted.sort_unstable_by(|x, y| x.0.cmp(&y.0));
                    sorted
                }
                by_key(a) == by_key(b)
            }
        }
    }
}

/// An absent key reads as null, as in PDF, instead of panicking as `BTreeMap`
/// indexing does.
impl<Q> std::ops::Index<&Q> for Dictionary
where
    Vec<u8>: Borrow<Q>,
    Q: Ord + ?Sized,
{
    type Output = Object;
    fn index(&self, key: &Q) -> &Object {
        static NULL: Object = Object::Null;
        self.get(key).unwrap_or(&NULL)
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
        let entries = entries.into_iter();
        let mut dictionary = Self::with_capacity(entries.size_hint().0);
        dictionary.extend(entries);
        dictionary
    }
}

impl<const N: usize> From<[(Vec<u8>, Object); N]> for Dictionary {
    fn from(entries: [(Vec<u8>, Object); N]) -> Self {
        entries.into_iter().collect()
    }
}

/// The map's entries in its (sorted) order.
impl From<BTreeMap<Vec<u8>, Object>> for Dictionary {
    fn from(map: BTreeMap<Vec<u8>, Object>) -> Self {
        map.into_iter().collect()
    }
}

impl IntoIterator for Dictionary {
    type Item = (Vec<u8>, Object);
    type IntoIter = IntoIter;
    fn into_iter(mut self) -> IntoIter {
        IntoIter(match self.entries.take() {
            Either::Ordered(entries) => Either::Ordered(entries.into_iter()),
            Either::Sorted(map) => Either::Sorted(map.into_iter()),
        })
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

/// An iterator over either kind of dictionary storage.
#[derive(Clone, Debug)]
enum Either<A, B> {
    Ordered(A),
    Sorted(B),
}
impl<A: Iterator, B: Iterator<Item = A::Item>> Iterator for Either<A, B> {
    type Item = A::Item;
    fn next(&mut self) -> Option<A::Item> {
        match self {
            Self::Ordered(a) => a.next(),
            Self::Sorted(b) => b.next(),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Ordered(a) => a.size_hint(),
            Self::Sorted(b) => b.size_hint(),
        }
    }
}
impl<A, B> DoubleEndedIterator for Either<A, B>
where
    A: DoubleEndedIterator,
    B: DoubleEndedIterator<Item = A::Item>,
{
    fn next_back(&mut self) -> Option<A::Item> {
        match self {
            Self::Ordered(a) => a.next_back(),
            Self::Sorted(b) => b.next_back(),
        }
    }
}

fn pair(slot: &Slot) -> Pair<'_> {
    (&slot.0, &slot.1)
}
fn pair_mut(slot: &mut Slot) -> PairMut<'_> {
    let (key, value) = slot;
    (key, value)
}
fn key(slot: &Slot) -> &Vec<u8> {
    &slot.0
}
fn value(slot: &Slot) -> &Object {
    &slot.1
}
fn value_mut(slot: &mut Slot) -> &mut Object {
    &mut slot.1
}
fn owned_key(slot: Slot) -> Vec<u8> {
    slot.0
}
fn owned_value(slot: Slot) -> Object {
    slot.1
}

type Pair<'a> = (&'a Vec<u8>, &'a Object);
type PairMut<'a> = (&'a Vec<u8>, &'a mut Object);
type Mapped<I, T> = std::iter::Map<I, fn(<I as Iterator>::Item) -> T>;
type SlotIter<'a> = std::slice::Iter<'a, Slot>;
type SlotIterMut<'a> = std::slice::IterMut<'a, Slot>;
type SlotIntoIter = std::vec::IntoIter<Slot>;

/// Iterators over a dictionary's entries in its order.
macro_rules! iterator {
    ($(#[$attr:meta])* $name:ident $(<$lt:lifetime>)?, $item:ty, $ordered:ty, $sorted:ty) => {
        $(#[$attr])*
        #[derive(Debug)]
        pub struct $name$(<$lt>)?(Either<$ordered, $sorted>);
        impl$(<$lt>)? Iterator for $name$(<$lt>)? {
            type Item = $item;
            fn next(&mut self) -> Option<$item> {
                self.0.next()
            }
            fn size_hint(&self) -> (usize, Option<usize>) {
                self.0.size_hint()
            }
        }
        impl$(<$lt>)? DoubleEndedIterator for $name$(<$lt>)? {
            fn next_back(&mut self) -> Option<$item> {
                self.0.next_back()
            }
        }
        impl$(<$lt>)? ExactSizeIterator for $name$(<$lt>)? {}
        impl$(<$lt>)? FusedIterator for $name$(<$lt>)? {}
    };
}
iterator!(
    #[derive(Clone)]
    Iter<'a>,
    Pair<'a>,
    Mapped<SlotIter<'a>, Pair<'a>>,
    btree_map::Iter<'a, Vec<u8>, Object>
);
iterator!(
    IterMut<'a>,
    PairMut<'a>,
    Mapped<SlotIterMut<'a>, PairMut<'a>>,
    btree_map::IterMut<'a, Vec<u8>, Object>
);
iterator!(
    #[derive(Clone)]
    Keys<'a>,
    &'a Vec<u8>,
    Mapped<SlotIter<'a>, &'a Vec<u8>>,
    btree_map::Keys<'a, Vec<u8>, Object>
);
iterator!(
    #[derive(Clone)]
    Values<'a>,
    &'a Object,
    Mapped<SlotIter<'a>, &'a Object>,
    btree_map::Values<'a, Vec<u8>, Object>
);
iterator!(
    ValuesMut<'a>,
    &'a mut Object,
    Mapped<SlotIterMut<'a>, &'a mut Object>,
    btree_map::ValuesMut<'a, Vec<u8>, Object>
);
iterator!(
    IntoIter,
    Slot,
    SlotIntoIter,
    btree_map::IntoIter<Vec<u8>, Object>
);
iterator!(
    IntoKeys,
    Vec<u8>,
    Mapped<SlotIntoIter, Vec<u8>>,
    btree_map::IntoKeys<Vec<u8>, Object>
);
iterator!(
    IntoValues,
    Object,
    Mapped<SlotIntoIter, Object>,
    btree_map::IntoValues<Vec<u8>, Object>
);

/// PDFDocEncoding codes whose Unicode value differs from the code itself
/// (ISO 32000-2 Annex D, Table D.2): 0x18..=0x1F and 0x80..=0xA0.
const PDF_DOC_SPECIAL: [(u8, char); 40] = [
    (0x18, '\u{02D8}'),
    (0x19, '\u{02C7}'),
    (0x1A, '\u{02C6}'),
    (0x1B, '\u{02D9}'),
    (0x1C, '\u{02DD}'),
    (0x1D, '\u{02DB}'),
    (0x1E, '\u{02DA}'),
    (0x1F, '\u{02DC}'),
    (0x80, '\u{2022}'),
    (0x81, '\u{2020}'),
    (0x82, '\u{2021}'),
    (0x83, '\u{2026}'),
    (0x84, '\u{2014}'),
    (0x85, '\u{2013}'),
    (0x86, '\u{0192}'),
    (0x87, '\u{2044}'),
    (0x88, '\u{2039}'),
    (0x89, '\u{203A}'),
    (0x8A, '\u{2212}'),
    (0x8B, '\u{2030}'),
    (0x8C, '\u{201E}'),
    (0x8D, '\u{201C}'),
    (0x8E, '\u{201D}'),
    (0x8F, '\u{2018}'),
    (0x90, '\u{2019}'),
    (0x91, '\u{201A}'),
    (0x92, '\u{2122}'),
    (0x93, '\u{FB01}'),
    (0x94, '\u{FB02}'),
    (0x95, '\u{0141}'),
    (0x96, '\u{0152}'),
    (0x97, '\u{0160}'),
    (0x98, '\u{0178}'),
    (0x99, '\u{017D}'),
    (0x9A, '\u{0131}'),
    (0x9B, '\u{0142}'),
    (0x9C, '\u{0153}'),
    (0x9D, '\u{0161}'),
    (0x9E, '\u{017E}'),
    (0xA0, '\u{20AC}'),
];

/// The Unicode value of a PDFDocEncoding byte. Codes the table leaves
/// undefined (controls, 0x7F, 0x9F, 0xAD) decode as their Latin-1 value.
fn pdf_doc_char(byte: u8) -> char {
    PDF_DOC_SPECIAL
        .iter()
        .find(|(code, _)| *code == byte)
        .map_or(char::from(byte), |(_, c)| *c)
}

/// The PDFDocEncoding byte of `c`, or `None` as in MuPDF for characters the
/// table lacks: U+0018..=U+001F, U+007F..=U+00A0 and U+00AD (codes it
/// redefines or leaves undefined) and everything beyond U+00FF it does not
/// list.
pub(crate) fn pdf_doc_byte(c: char) -> Option<u8> {
    if let Some((code, _)) = PDF_DOC_SPECIAL.iter().find(|(_, special)| *special == c) {
        return Some(*code);
    }
    let byte = u8::try_from(u32::from(c)).ok()?;
    (!matches!(byte, 0x18..=0x1F | 0x7F..=0xA0 | 0xAD)).then_some(byte)
}

/// Encode a text string as MuPDF's `pdf_new_text_string` does: the bytes
/// themselves when every character is ASCII, otherwise UTF-16BE with a byte
/// order mark. ASCII 0x18..=0x1F, which PDFDocEncoding reads as accents, also
/// takes UTF-16 so that `decode_text_string` always returns the text.
pub fn encode_text_string(text: &str) -> Vec<u8> {
    if text
        .chars()
        .all(|c| c.is_ascii() && !('\u{18}'..='\u{1F}').contains(&c))
    {
        return text.as_bytes().to_vec();
    }
    let mut bytes = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes
}

/// Decode a text string as MuPDF reads one (`pdf_new_utf8_from_pdf_string`):
/// UTF-16BE, UTF-16LE or UTF-8 when it starts with that byte order mark;
/// otherwise UTF-8 when the bytes are valid UTF-8 holding a non-ASCII
/// character and none of 0x18..=0x1F (the codes PDFDocEncoding reads as
/// accents), else PDFDocEncoding. Malformed UTF-16 or UTF-8 after a mark
/// becomes U+FFFD; an odd trailing UTF-16 byte is ignored. Unlike MuPDF,
/// overlong forms, encoded surrogates and code points past U+10FFFF are not
/// UTF-8 (MuPDF passes such bytes on as they are), and PDFDocEncoding's
/// undefined codes keep their Latin-1 value (MuPDF reads U+0000).
pub fn decode_text_string(bytes: &[u8]) -> String {
    decode_text(bytes, bytes.len())
}

/// `decode_text_string` of at most `max` bytes, and whether bytes were
/// left out. The whole string picks the encoding, as it does uncut; a
/// character the cut splits is left out.
pub fn decode_text_prefix(bytes: &[u8], max: usize) -> (String, bool) {
    (decode_text(bytes, max), bytes.len() > max)
}

fn decode_text(bytes: &[u8], max: usize) -> String {
    let cut = bytes.len() > max;
    let head = bytes.get(..max).unwrap_or(bytes);
    // What follows a mark of `mark` bytes, within the first `max`.
    let body = |mark: usize| head.get(mark..).unwrap_or(&[]);
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let mut units: Vec<u16> = rest.chunks_exact(2).map(|c| unit([c[0], c[1]])).collect();
        if cut
            && units
                .last()
                .is_some_and(|unit| (0xD800..0xDC00).contains(unit))
        {
            units.pop();
        }
        String::from_utf16_lossy(&units)
    };
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return utf16(body(2), u16::from_be_bytes);
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return utf16(body(2), u16::from_le_bytes);
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let rest = body(3);
        return match utf8(rest, cut) {
            Some(text) => text.to_string(),
            None => String::from_utf8_lossy(rest).into_owned(),
        };
    }
    if unmarked_utf8(bytes) {
        if let Some(text) = utf8(head, cut) {
            return text.to_string();
        }
    }
    head.iter().map(|&b| pdf_doc_char(b)).collect()
}

/// MuPDF's test for a text string without a mark: valid UTF-8 holding a
/// non-ASCII character and no byte PDFDocEncoding reads as an accent.
fn unmarked_utf8(bytes: &[u8]) -> bool {
    !bytes.is_ascii()
        && !bytes.iter().any(|b| (0x18..=0x1F).contains(b))
        && std::str::from_utf8(bytes).is_ok()
}

/// `bytes` as UTF-8, if valid; when `cut`, an incomplete last character is
/// left out.
fn utf8(bytes: &[u8], cut: bool) -> Option<&str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(error) if cut && error.error_len().is_none() => {
            std::str::from_utf8(bytes.get(..error.valid_up_to())?).ok()
        }
        Err(_) => None,
    }
}
