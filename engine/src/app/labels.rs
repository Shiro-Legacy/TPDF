// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page labels: the catalog's /PageLabels number tree, label text as MuPDF
//! formats it (`pdf_page_label`), and the edits page and label ops make.
//!
//! Reading takes every /Nums pair of the tree, nested /Kids included; the
//! label of a page comes from the entry with the greatest key at or below it
//! (the later of equal keys). Writing flattens the tree into one sorted
//! /Nums array with distinct keys, as MuPDF's writes flatten it.
use std::collections::BTreeSet;

use crate::document::{Document, DocumentError};
use crate::object::{Dictionary, Object, ObjectId, decode_text_prefix};

use super::edit::{Edit, Place, get};

/// MuPDF formats labels into a 100-byte buffer: at most 99 bytes.
const MAX_LABEL_BYTES: usize = 99;

/// engine.ts `LABEL_STYLES`: decimal, roman, letters; "" is a prefix only.
pub const STYLES: [&str; 6] = ["D", "r", "R", "a", "A", ""];

/// One range: its first page and its label dictionary (as stored).
pub type Entry = (i64, Object);

/// The tree's entries sorted by key, the later of equal keys last.
pub fn entries(doc: &Document<'_>) -> Option<Vec<Entry>> {
    let root = doc.resolve_id(doc.root());
    let tree = get(doc, root, b"PageLabels");
    tree.as_dict()?;
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![(tree, 0usize)];
    // Depth-first in tree order: a node's /Kids, then its /Nums.
    let mut order: Vec<&Object> = Vec::new();
    while let Some((node, depth)) = stack.pop() {
        if let Some(id) = holder(doc, node) {
            if !seen.insert(id) {
                continue;
            }
        }
        let Some(dict) = doc.resolve(node).as_dict() else {
            continue;
        };
        order.push(node);
        if depth < 64 {
            if let Some(kids) = doc.dict_get(dict, b"Kids").as_array() {
                stack.extend(kids.iter().rev().map(|kid| (kid, depth + 1)));
            }
        }
    }
    for node in order {
        let Some(nums) = get(doc, node, b"Nums").as_array() else {
            continue;
        };
        for pair in nums.chunks_exact(2) {
            if let [key, value] = pair {
                if let Some(key) = number(doc.resolve(key)) {
                    entries.push((key, value.clone()));
                }
            }
        }
    }
    entries.sort_by_key(|(key, _)| *key);
    Some(entries)
}

/// The object a value is held in, when it is a reference.
fn holder(doc: &Document<'_>, value: &Object) -> Option<ObjectId> {
    doc.resolve_with_holder(value).0
}

/// MuPDF's `pdf_to_int`: integers, and reals rounded (`floor(x + 0.5)`),
/// saturating.
fn number(value: &Object) -> Option<i64> {
    match value {
        Object::Integer(value) => Some(*value),
        Object::Real(value) if value.is_finite() => Some((value + 0.5).floor() as i64),
        _ => None,
    }
}

/// Every page's label, `""` where the document gives none (engine.ts then
/// shows the 1-based index).
pub fn page_labels(doc: &Document<'_>, count: usize) -> Vec<String> {
    let Some(entries) = entries(doc) else {
        return (1..=count).map(|page| page.to_string()).collect();
    };
    let mut labels = Vec::with_capacity(count);
    let mut next = 0;
    let mut current: Option<(i64, Range)> = None;
    for page in 0..count {
        let index = i64::try_from(page).unwrap_or(i64::MAX);
        let mut found = None;
        while let Some(entry) = entries.get(next).filter(|(key, _)| *key <= index) {
            found = Some(entry);
            next += 1;
        }
        // Each range is read once, when it starts.
        if let Some((key, dict)) = found {
            current = Some((*key, Range::read(doc, dict)));
        }
        labels.push(match &current {
            Some((key, range)) => range.format(index.saturating_sub(*key)),
            None => (page + 1).to_string(),
        });
    }
    labels
}

/// engine.ts `pageLabels`: the label, or the 1-based index when it is empty.
pub fn items(doc: &Document<'_>, count: usize) -> Vec<String> {
    let mut labels = page_labels(doc, count);
    for (page, label) in labels.iter_mut().enumerate() {
        if label.is_empty() {
            *label = (page + 1).to_string();
        }
    }
    labels
}

/// A label range's dictionary, read.
struct Range {
    /// The prefix, cut to `MAX_LABEL_BYTES`.
    prefix: String,
    /// What the number may take after the whole prefix.
    room: usize,
    style: Option<Vec<u8>>,
    start: i64,
}

impl Range {
    /// /St below 1 counts as 1. Only the start of /P is decoded: at most
    /// `MAX_LABEL_BYTES` of it are used, and each unit decodes to a byte or more.
    fn read(doc: &Document<'_>, dict: &Object) -> Self {
        let prefix = match get(doc, dict, b"P") {
            Object::String(bytes) => decode_text_prefix(bytes, PREFIX_BYTES).0,
            _ => String::new(),
        };
        Self {
            room: MAX_LABEL_BYTES.saturating_sub(prefix.len()),
            prefix: cut(prefix),
            style: get(doc, dict, b"S").as_name().map(<[u8]>::to_vec),
            start: number(get(doc, dict, b"St")).unwrap_or(1).max(1),
        }
    }

    /// MuPDF's `pdf_format_page_label`: the prefix, then `offset + /St` in
    /// the style.
    fn format(&self, offset: i64) -> String {
        let (n, room) = (self.start.saturating_add(offset), self.room);
        if room == 0 {
            // The whole prefix already fills the label.
            return self.prefix.clone();
        }
        let number = match self.style.as_deref() {
            Some(b"D") => n.to_string(),
            Some(b"R") => roman(n, true, room),
            Some(b"r") => roman(n, false, room),
            Some(b"A") => letters(n, b'A', room),
            Some(b"a") => letters(n, b'a', room),
            _ => String::new(),
        };
        cut(self.prefix.clone() + &number)
    }
}

/// Raw /P bytes decoded: a byte-order mark and two bytes for each of
/// twice `MAX_LABEL_BYTES` units, more than any label keeps.
const PREFIX_BYTES: usize = 4 * MAX_LABEL_BYTES + 3;

/// `label` cut to `MAX_LABEL_BYTES` on a character boundary.
fn cut(mut label: String) -> String {
    if label.len() > MAX_LABEL_BYTES {
        let mut end = MAX_LABEL_BYTES;
        while !label.is_char_boundary(end) {
            end -= 1;
        }
        label.truncate(end);
    }
    label
}

/// Roman numerals as MuPDF writes them: one M per thousand.
fn roman(n: i64, upper: bool, room: usize) -> String {
    const ONES: [&str; 10] = ["", "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix"];
    const TENS: [&str; 10] = ["", "x", "xx", "xxx", "xl", "l", "lx", "lxx", "lxxx", "xc"];
    const HUNDREDS: [&str; 10] = ["", "c", "cc", "ccc", "cd", "d", "dc", "dcc", "dccc", "cm"];
    let n = n.max(0);
    let digit = |scale: i64| usize::try_from((n / scale) % 10).unwrap_or(0);
    let thousands = usize::try_from(n / 1000).unwrap_or(usize::MAX).min(room);
    let mut text = "m".repeat(thousands);
    for part in [HUNDREDS[digit(100)], TENS[digit(10)], ONES[digit(1)]] {
        text.push_str(part);
    }
    text.truncate(room);
    if upper { text.to_uppercase() } else { text }
}

/// Letters as MuPDF writes them: A..Z, then AA..ZZ, then AAA...
fn letters(n: i64, base: u8, room: usize) -> String {
    let n = n.max(1) - 1;
    let letter = char::from(base + u8::try_from(n % 26).unwrap_or(0));
    let reps = usize::try_from(n / 26 + 1).unwrap_or(usize::MAX).min(room);
    std::iter::repeat_n(letter, reps).collect()
}

/// The label dictionary `labels-set` writes: /S unless the style is "",
/// /P unless the prefix is "", /St unless the start is 1.
pub fn label(style: &str, prefix: &str, start: i64) -> Object {
    let mut dict = Dictionary::new();
    if !style.is_empty() {
        dict.insert(b"S".to_vec(), Object::Name(style.as_bytes().to_vec()));
    }
    if !prefix.is_empty() {
        dict.insert(b"P".to_vec(), Object::text(prefix));
    }
    if start != 1 {
        dict.insert(b"St".to_vec(), Object::Integer(start));
    }
    Object::Dictionary(dict)
}

fn decimal() -> Object {
    label("D", "", 1)
}

/// Entries after `labels-set`: a range from `from`, and a decimal range
/// from page 0 when nothing covered it.
pub fn set(mut entries: Vec<Entry>, from: i64, value: Object) -> Vec<Entry> {
    if from > 0 && !entries.iter().any(|(key, _)| *key == 0) {
        entries.push((0, decimal()));
    }
    entries.retain(|(key, _)| *key != from);
    entries.push((from, value));
    normalized(entries)
}

/// Entries after `labels-clear`: the range starting at `from` goes; the
/// one at page 0 becomes decimal, since page 0 must be covered.
pub fn clear(mut entries: Vec<Entry>, from: i64) -> Vec<Entry> {
    if from == 0 {
        entries.retain(|(key, _)| *key != 0);
        entries.push((0, decimal()));
    } else {
        entries.retain(|(key, _)| *key != from);
    }
    normalized(entries)
}

/// Entries after `count` pages are inserted at `at`: the inserted pages
/// join the range before them (the first range when `at` is 0).
pub fn inserted(entries: Vec<Entry>, at: i64, count: i64) -> Vec<Entry> {
    let shifted = entries
        .into_iter()
        .map(|(key, value)| {
            let moves = key > at || (key == at && at > 0);
            (
                if moves {
                    key.saturating_add(count)
                } else {
                    key
                },
                value,
            )
        })
        .collect();
    normalized(shifted)
}

/// Entries after page `index` is deleted, leaving `count` pages: later
/// ranges move up; a range that started at the deleted page gives way to
/// one starting at the next page; ranges past the end go (not page 0's).
pub fn deleted(entries: Vec<Entry>, index: i64, count: i64) -> Vec<Entry> {
    let shifted = entries
        .into_iter()
        .map(|(key, value)| (if key > index { key - 1 } else { key }, value))
        .filter(|(key, _)| *key == 0 || *key < count)
        .collect();
    normalized(shifted)
}

/// Sorted by key; of equal keys the last one stays.
pub fn normalized(mut entries: Vec<Entry>) -> Vec<Entry> {
    entries.sort_by_key(|(key, _)| *key);
    let mut out: Vec<Entry> = Vec::with_capacity(entries.len());
    for entry in entries {
        match out.last_mut() {
            Some(last) if last.0 == entry.0 => *last = entry,
            _ => out.push(entry),
        }
    }
    out
}

/// Writes `entries` as the catalog's /PageLabels: one sorted /Nums array.
/// An indirect /PageLabels object is rewritten in place.
pub fn write<E: Edit>(target: &mut E, entries: Vec<Entry>) -> Result<(), DocumentError> {
    let doc = target.document();
    let place = Place::object(doc.root()).child(doc, b"PageLabels");
    let nums = entries
        .into_iter()
        .flat_map(|(key, value)| [Object::Integer(key), value]);
    let nums = Object::Array(nums.collect());
    place.update(target, |value| {
        if !matches!(value, Object::Dictionary(_)) {
            *value = Object::Dictionary(Dictionary::new());
        }
        if let Some(dict) = value.as_dict_mut() {
            dict.remove(b"Kids".as_slice());
            dict.remove(b"Limits".as_slice());
            dict.insert(b"Nums".to_vec(), nums);
        }
    })
}

/// Rewrites the labels with `change`, when the document has any and the
/// ranges change. Returns whether it wrote.
pub fn adjust<E: Edit>(
    target: &mut E,
    change: impl FnOnce(Vec<Entry>) -> Vec<Entry>,
) -> Result<bool, DocumentError> {
    let Some(before) = entries(target.document()) else {
        return Ok(false);
    };
    let after = change(before.clone());
    if after == normalized(before) {
        return Ok(false);
    }
    write(target, after)?;
    Ok(true)
}
