// SPDX-License-Identifier: AGPL-3.0-or-later
//! Font facts the edit scanner needs (content-edit.ts `editResolveFont`):
//! the reply's font summary, how a simple font's codes map to text, and the
//! proven code maps of Identity-H and "coded" symbolic simple fonts (parsed
//! /ToUnicode, /W or /Widths) whose writes stay inside the page repertoire.
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::simple_encoding::{GlyphUnicode, NamedEncoding, SimpleEncoding};

use super::{Budget, MAX_EDIT_SCAN_BYTES, decode};

/// Every code a /ToUnicode CMap or /W may expand to (content-edit.ts `EDIT_MAX_CMAP_CODES`).
pub const MAX_CMAP_CODES: usize = 1 << 20;

/// The reply's font object; every field null for an unresolved font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontSummary {
    pub name: Option<String>,
    pub subtype: Option<String>,
    pub embedded: Option<bool>,
    pub subset: Option<bool>,
    pub writing_mode: Option<f64>,
}

impl FontSummary {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name, "subtype": self.subtype, "embedded": self.embedded,
            "subset": self.subset, "writingMode": self.writing_mode,
        })
    }
}

/// How a simple font's one-byte codes read as text and how text is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Codec {
    /// `/Encoding /WinAnsiEncoding`.
    WinAnsi,
    /// `/MacRomanEncoding` or `/StandardEncoding`: decoded and written
    /// through that table (engine.ts read them as WinAnsi and wrote ASCII).
    Named(NamedEncoding),
    /// No usable named encoding: codes read as WinAnsi, writes are
    /// printable ASCII only (engine.ts's rule).
    #[default]
    Ascii,
}

impl Codec {
    /// The text of `bytes`, or None when a byte has no character.
    pub fn decode(self, bytes: &[u8]) -> Option<String> {
        match self {
            Self::WinAnsi | Self::Ascii => bytes.iter().map(|&b| win_ansi_char(b)).collect(),
            Self::Named(encoding) => {
                let table = SimpleEncoding::from_named(encoding);
                bytes
                    .iter()
                    .map(|&b| match table.lookup_unicode(b) {
                        GlyphUnicode::Unicode([c]) if !c.is_control() => Some(*c),
                        _ => None,
                    })
                    .collect()
            }
        }
    }

    /// The code of `c`, or None when this codec cannot write it.
    pub fn encode(self, c: char) -> Option<u8> {
        match self {
            Self::WinAnsi => win_ansi_byte(c),
            Self::Ascii => u8::try_from(u32::from(c))
                .ok()
                .filter(|b| (0x20..=0x7e).contains(b)),
            Self::Named(encoding) => {
                if c < ' ' {
                    return None;
                }
                let table = SimpleEncoding::from_named(encoding);
                (0..=255u8).find(
                    |&b| matches!(table.lookup_unicode(b), GlyphUnicode::Unicode([d]) if *d == c),
                )
            }
        }
    }

    pub fn is_win_ansi(self) -> bool {
        self == Self::WinAnsi
    }
}

/// engine-core.ts `WIN_ANSI_EXTRA`: 0x80-0x9F.
const WIN_ANSI_EXTRA: [(u32, u8); 27] = [
    (0x20ac, 0x80),
    (0x201a, 0x82),
    (0x0192, 0x83),
    (0x201e, 0x84),
    (0x2026, 0x85),
    (0x2020, 0x86),
    (0x2021, 0x87),
    (0x02c6, 0x88),
    (0x2030, 0x89),
    (0x0160, 0x8a),
    (0x2039, 0x8b),
    (0x0152, 0x8c),
    (0x017d, 0x8e),
    (0x2018, 0x91),
    (0x2019, 0x92),
    (0x201c, 0x93),
    (0x201d, 0x94),
    (0x2022, 0x95),
    (0x2013, 0x96),
    (0x2014, 0x97),
    (0x02dc, 0x98),
    (0x2122, 0x99),
    (0x0161, 0x9a),
    (0x203a, 0x9b),
    (0x0153, 0x9c),
    (0x017e, 0x9e),
    (0x0178, 0x9f),
];

/// engine-core.ts `winAnsiByte`.
pub fn win_ansi_byte(c: char) -> Option<u8> {
    let code = u32::from(c);
    if (0x20..=0x7e).contains(&code) || (0xa0..=0xff).contains(&code) {
        return u8::try_from(code).ok();
    }
    WIN_ANSI_EXTRA
        .iter()
        .find(|(unicode, _)| *unicode == code)
        .map(|(_, byte)| *byte)
}

/// engine-core.ts `winAnsiChar`.
pub fn win_ansi_char(byte: u8) -> Option<char> {
    if (0x20..=0x7e).contains(&byte) || byte >= 0xa0 {
        return Some(char::from(byte));
    }
    WIN_ANSI_EXTRA
        .iter()
        .find(|(_, b)| *b == byte)
        .and_then(|(unicode, _)| char::from_u32(*unicode))
}

/// A proven code map (content-edit.ts `EditCid`): Identity-H (two bytes per
/// code) or a coded symbolic simple font (one byte).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cid {
    pub to_unicode: BTreeMap<u32, char>,
    /// Advances in 1000-unit text space.
    pub widths: BTreeMap<u32, f64>,
    pub dw: f64,
    pub bytes_per_code: usize,
}

impl Cid {
    /// content-edit.ts `editCodesOf`: an odd tail of a two-byte payload has no code.
    pub fn codes(&self, bytes: &[u8]) -> Vec<u32> {
        codes_of(bytes, self.bytes_per_code)
    }

    /// content-edit.ts `editDecodeCid`.
    pub fn decode(&self, bytes: &[u8]) -> Option<(String, Vec<u32>)> {
        if self.bytes_per_code == 2 && bytes.len() % 2 != 0 {
            return None;
        }
        let codes = self.codes(bytes);
        let text = codes
            .iter()
            .map(|code| self.to_unicode.get(code).copied())
            .collect::<Option<String>>()?;
        Some((text, codes))
    }

    pub fn width(&self, code: u32) -> f64 {
        self.widths.get(&code).copied().unwrap_or(self.dw)
    }
}

pub fn codes_of(bytes: &[u8], bytes_per_code: usize) -> Vec<u32> {
    if bytes_per_code == 1 {
        return bytes.iter().map(|&b| u32::from(b)).collect();
    }
    bytes
        .chunks_exact(2)
        .map(|pair| (u32::from(pair[0]) << 8) | u32::from(pair[1]))
        .collect()
}

/// One resolved font dictionary.
#[derive(Clone, Debug, Default)]
pub struct Font {
    pub summary: FontSummary,
    pub subtype: Option<Vec<u8>>,
    pub subset: Option<bool>,
    pub simple: bool,
    /// `/Encoding` is a dictionary (Differences).
    pub encoding_unknown: bool,
    pub codec: Codec,
    pub writing_mode: Option<f64>,
    pub cid: Option<Arc<Cid>>,
    /// The font dictionary's object: the repertoire's identity.
    pub object: Option<ObjectId>,
}

impl Font {
    pub fn is_subtype(&self, name: &[u8]) -> bool {
        self.subtype.as_deref() == Some(name)
    }
}

pub fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// `^[A-Z]{6}\+`.
pub fn has_subset_prefix(name: &[u8]) -> bool {
    name.len() > 6 && name[..6].iter().all(u8::is_ascii_uppercase) && name[6] == b'+'
}

fn strip_subset(name: &[u8]) -> &[u8] {
    if has_subset_prefix(name) {
        &name[7..]
    } else {
        name
    }
}

/// MuPDF's `at(object, key)`: the resolved value, None for null or absent.
pub fn at<'d>(
    doc: &'d Document<'_>,
    dict: Option<&'d Dictionary>,
    key: &[u8],
) -> Option<&'d Object> {
    let value = doc.dict_get(dict?, key);
    (!value.is_null()).then_some(value)
}

pub fn dict_at<'d>(
    doc: &'d Document<'_>,
    dict: Option<&'d Dictionary>,
    key: &[u8],
) -> Option<&'d Dictionary> {
    at(doc, dict, key)?.as_dict()
}

pub fn number(value: Option<&Object>) -> Option<f64> {
    match value? {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) => Some(*v),
        _ => None,
    }
}

fn is_stream(doc: &Document<'_>, value: Option<&Object>) -> Option<ObjectId> {
    let id = value?.as_reference()?;
    let (holder, _) = doc.resolve_id_with_holder(id);
    let holder = holder?;
    doc.object(holder)
        .ok()
        .filter(|object| object.stream.is_some())
        .map(|_| holder)
}

/// Font resolution with the scan's cache and budget.
#[derive(Default)]
pub struct Fonts {
    cache: BTreeMap<ObjectId, Option<Arc<Font>>>,
}

impl Fonts {
    /// content-edit.ts `editResolveFont`: `/Font /<name>` in `resources`.
    pub fn resolve(
        &mut self,
        doc: &Document<'_>,
        resources: Option<&Dictionary>,
        name: Option<&[u8]>,
        budget: &mut Budget,
    ) -> Option<Arc<Font>> {
        let fonts = dict_at(doc, resources, b"Font")?;
        let entry = fonts.get(name?)?;
        let (holder, value) = doc.resolve_with_holder(entry);
        let font = value.as_dict()?;
        let object = holder.filter(|_| entry.as_reference().is_some());
        if let Some(object) = object {
            if let Some(cached) = self.cache.get(&object) {
                return cached.clone();
            }
        }
        let resolved = Some(Arc::new(resolve_font(doc, font, object, budget)));
        if let Some(object) = object {
            self.cache.insert(object, resolved.clone());
        }
        resolved
    }
}

fn resolve_font(
    doc: &Document<'_>,
    font: &Dictionary,
    object: Option<ObjectId>,
    budget: &mut Budget,
) -> Font {
    let name_of = |key: &[u8]| at(doc, Some(font), key).and_then(Object::as_name);
    let subtype = name_of(b"Subtype").map(<[u8]>::to_vec);
    let base_font = name_of(b"BaseFont");
    let encoding = at(doc, Some(font), b"Encoding");
    let encoding_unknown = encoding.is_some_and(|e| e.as_dict().is_some());
    let codec = match encoding.and_then(Object::as_name) {
        Some(b"WinAnsiEncoding") => Codec::WinAnsi,
        Some(b"MacRomanEncoding") => Codec::Named(NamedEncoding::MacRoman),
        Some(b"StandardEncoding") => Codec::Named(NamedEncoding::Standard),
        _ => Codec::Ascii,
    };
    let simple = matches!(
        subtype.as_deref(),
        Some(b"Type1" | b"TrueType" | b"MMType1")
    );
    let type0 = subtype.as_deref() == Some(b"Type0");
    let mut writing_mode = None;
    let mut descriptor = dict_at(doc, Some(font), b"FontDescriptor");
    if type0 {
        let first = at(doc, Some(font), b"DescendantFonts")
            .and_then(Object::as_array)
            .and_then(|items| items.first())
            .and_then(|first| doc.resolve(first).as_dict());
        if let Some(first) = first {
            // The writing mode belongs to the CMap (Identity-V); engine.ts read
            // only the descendant's non-standard /WMode, which still counts.
            let declared = number(at(doc, Some(first), b"WMode")).unwrap_or(0.0);
            let cmap_vertical = match encoding {
                Some(Object::Name(name)) => name.ends_with(b"-V"),
                Some(other) => {
                    doc.resolve_dict(other)
                        .and_then(|cmap| number(at(doc, Some(cmap), b"WMode")))
                        == Some(1.0)
                }
                None => false,
            };
            writing_mode = Some(if cmap_vertical { 1.0 } else { declared });
            descriptor = dict_at(doc, Some(first), b"FontDescriptor").or(descriptor);
        }
    }
    let has_program = |d: &Dictionary| {
        [&b"FontFile"[..], b"FontFile2", b"FontFile3"]
            .iter()
            .any(|key| at(doc, Some(d), key).is_some())
    };
    let embedded = match descriptor {
        Some(d) => Some(has_program(d)),
        None if simple => Some(false),
        None => None,
    };
    let cid = if type0 {
        resolve_cid(doc, font, budget)
    } else if simple {
        resolve_coded(doc, font, descriptor, budget)
    } else {
        None
    };
    let subset = base_font.map(has_subset_prefix);
    let name = base_font.map(|b| latin1(strip_subset(b)));
    Font {
        summary: FontSummary {
            name,
            subtype: subtype.as_deref().map(latin1),
            embedded,
            subset,
            writing_mode,
        },
        subtype,
        subset,
        simple,
        encoding_unknown,
        codec,
        writing_mode,
        cid: cid.map(Arc::new),
        object,
    }
}

/// content-edit.ts `editReadBounded`: a stream's decoded bytes inside the
/// scan budget, None when unreadable or over it (the font stays unproven).
fn read_bounded(doc: &Document<'_>, id: ObjectId, budget: &mut Budget) -> Option<Vec<u8>> {
    let room = MAX_EDIT_SCAN_BYTES.saturating_sub(budget.bytes);
    let bytes = decode(doc, id, room).ok()?;
    if budget.bytes + bytes.len() > MAX_EDIT_SCAN_BYTES {
        return None;
    }
    budget.bytes += bytes.len();
    Some(bytes)
}

/// content-edit.ts `editResolveCid`: a fully proven Identity-H font.
fn resolve_cid(doc: &Document<'_>, font: &Dictionary, budget: &mut Budget) -> Option<Cid> {
    if at(doc, Some(font), b"Encoding")?.as_name()? != b"Identity-H" {
        return None;
    }
    let descendants = at(doc, Some(font), b"DescendantFonts")?.as_array()?;
    let [descendant] = descendants else {
        return None;
    };
    let descendant = doc.resolve(descendant).as_dict()?;
    let subtype = at(doc, Some(descendant), b"Subtype")?.as_name()?;
    if subtype != b"CIDFontType2" && subtype != b"CIDFontType0" {
        return None;
    }
    if let Some(map) = at(doc, Some(descendant), b"CIDToGIDMap") {
        if map.as_name() != Some(b"Identity") {
            return None;
        }
    }
    let descriptor = dict_at(doc, Some(descendant), b"FontDescriptor")
        .or_else(|| dict_at(doc, Some(font), b"FontDescriptor"))?;
    if at(doc, Some(descriptor), b"FontFile2").is_none()
        && at(doc, Some(descriptor), b"FontFile3").is_none()
    {
        return None;
    }
    let cmap = is_stream(doc, font.get(b"ToUnicode".as_slice()))?;
    let bytes = read_bounded(doc, cmap, budget)?;
    let to_unicode = parse_to_unicode(&bytes, 2)?;
    let (widths, dw) = parse_widths(doc, descendant)?;
    Some(Cid {
        to_unicode,
        widths,
        dw,
        bytes_per_code: 2,
    })
}

/// content-edit.ts `editResolveCoded`: a symbolic simple font with no
/// /Encoding whose one-byte /ToUnicode, /Widths and program are proven.
fn resolve_coded(
    doc: &Document<'_>,
    font: &Dictionary,
    descriptor: Option<&Dictionary>,
    budget: &mut Budget,
) -> Option<Cid> {
    if at(doc, Some(font), b"Encoding").is_some() {
        return None;
    }
    let descriptor = descriptor?;
    let flags = number(at(doc, Some(descriptor), b"Flags"))?.trunc() as i64;
    if flags & 4 == 0 || flags & 32 != 0 {
        return None;
    }
    if ![&b"FontFile"[..], b"FontFile2", b"FontFile3"]
        .iter()
        .any(|key| at(doc, Some(descriptor), key).is_some())
    {
        return None;
    }
    let cmap = is_stream(doc, font.get(b"ToUnicode".as_slice()))?;
    let first = number(at(doc, Some(font), b"FirstChar")).filter(|f| f.fract() == 0.0)?;
    let list = at(doc, Some(font), b"Widths")?.as_array()?;
    let mut widths = BTreeMap::new();
    for (index, item) in list.iter().enumerate() {
        let code = first + index as f64;
        if code > 255.0 {
            break;
        }
        let width = number(Some(doc.resolve(item)))?;
        if code >= 0.0 {
            widths.insert(code as u32, width);
        }
    }
    let bytes = read_bounded(doc, cmap, budget)?;
    let to_unicode = parse_to_unicode(&bytes, 1)?;
    let dw = number(at(doc, Some(descriptor), b"MissingWidth")).unwrap_or(0.0);
    Some(Cid {
        to_unicode,
        widths,
        dw,
        bytes_per_code: 1,
    })
}

/// content-edit.ts `editParseWidths`: `/W` in both forms and `/DW`.
fn parse_widths(doc: &Document<'_>, descendant: &Dictionary) -> Option<(BTreeMap<u32, f64>, f64)> {
    let list = at(doc, Some(descendant), b"W")?.as_array()?;
    let item = |index: usize| list.get(index).map(|value| doc.resolve(value));
    let mut widths = BTreeMap::new();
    let mut generated = 0usize;
    let mut index = 0;
    while index < list.len() {
        let first = number(item(index))?;
        index += 1;
        let next = item(index)?;
        if let Some(array) = next.as_array() {
            for (offset, width) in array.iter().enumerate() {
                let width = number(Some(doc.resolve(width)))?;
                let code = first.trunc() + offset as f64;
                if (0.0..=65535.0).contains(&code) {
                    widths.insert(code as u32, width);
                }
            }
            index += 1;
            continue;
        }
        let high = number(Some(next))?;
        let width = number(item(index + 1))?;
        let (low, high) = (first.trunc(), high.trunc());
        if high < low || high - low > 65535.0 {
            return None;
        }
        let (from, to) = (low.max(0.0), high.min(65535.0));
        if to >= from {
            generated += (to - from) as usize + 1;
            if generated > MAX_CMAP_CODES {
                return None;
            }
            for code in from as u32..=to as u32 {
                widths.insert(code, width);
            }
        }
        index += 2;
    }
    let dw = match at(doc, Some(descendant), b"DW") {
        None => 1000.0,
        Some(value) => number(Some(value))?,
    };
    Some((widths, dw))
}

fn is_hex(text: &[u8], digits: usize) -> bool {
    text.len() == digits && text.iter().all(u8::is_ascii_hexdigit)
}

fn hex_value(text: &[u8]) -> u32 {
    text.iter().fold(0u32, |value, &b| {
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            _ => b.wrapping_sub(b'A').wrapping_add(10),
        };
        value.wrapping_mul(16).wrapping_add(u32::from(digit))
    })
}

/// JavaScript `\s` over Latin-1 text.
fn is_js_space(b: u8) -> bool {
    matches!(b, 9..=13 | b' ' | 0xa0)
}

/// `<hex>` at `at`: the digits and the end.
fn hex_token(text: &[u8], at: usize) -> Option<(&[u8], usize)> {
    if text.get(at) != Some(&b'<') {
        return None;
    }
    let start = at + 1;
    let mut end = start;
    while text.get(end).is_some_and(u8::is_ascii_hexdigit) {
        end += 1;
    }
    (end > start && text.get(end) == Some(&b'>')).then(|| (&text[start..end], end + 1))
}

fn skip_space(text: &[u8], mut at: usize) -> usize {
    while text.get(at).copied().is_some_and(is_js_space) {
        at += 1;
    }
    at
}

/// Every `<a>\s*<b>` match, as a global regex finds them.
fn hex_pairs(text: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        if let Some((a, end)) = hex_token(text, at) {
            if let Some((b, end)) = hex_token(text, skip_space(text, end)) {
                out.push((a, b));
                at = end;
                continue;
            }
        }
        at += 1;
    }
    out
}

/// The bodies between each `begin` and the next `end`, non-greedily.
fn blocks<'t>(text: &'t [u8], begin: &[u8], end: &[u8]) -> Vec<&'t [u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(start) = find(&text[at..], begin).map(|i| at + i + begin.len()) {
        let Some(stop) = find(&text[start..], end).map(|i| start + i) else {
            break;
        };
        out.push(&text[start..stop]);
        at = stop + end.len();
    }
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `/\busecmap\b/`.
fn mentions_usecmap(text: &[u8]) -> bool {
    let needle = b"usecmap";
    let mut at = 0;
    while let Some(i) = find(&text[at..], needle).map(|i| at + i) {
        let before = i.checked_sub(1).and_then(|p| text.get(p)).copied();
        let after = text.get(i + needle.len()).copied();
        if !before.is_some_and(is_word) && !after.is_some_and(is_word) {
            return true;
        }
        at = i + 1;
    }
    false
}

/// content-edit.ts `editParseToUnicode`: one- or two-byte sources only, a
/// destination of exactly four hex digits is one BMP character (others leave
/// the code unmapped); anything it cannot follow is None, never a partial map.
pub fn parse_to_unicode(text: &[u8], bytes_per_code: usize) -> Option<BTreeMap<u32, char>> {
    if mentions_usecmap(text) {
        return None;
    }
    let digits = bytes_per_code * 2;
    let mut seen_space = false;
    for block in blocks(text, b"begincodespacerange", b"endcodespacerange") {
        for (low, high) in hex_pairs(block) {
            seen_space = true;
            if !is_hex(low, digits) || !is_hex(high, digits) {
                return None;
            }
        }
    }
    if !seen_space {
        return None;
    }
    let mut map = BTreeMap::new();
    let add = |map: &mut BTreeMap<u32, char>, code: u32, destination: &[u8]| {
        if is_hex(destination, 4) {
            if let Some(c) = char::from_u32(hex_value(destination)) {
                map.insert(code, c);
            }
        }
    };
    for block in blocks(text, b"beginbfchar", b"endbfchar") {
        for (source, destination) in hex_pairs(block) {
            if !is_hex(source, digits) {
                return None;
            }
            add(&mut map, hex_value(source), destination);
        }
    }
    let mut generated = 0usize;
    for body in blocks(text, b"beginbfrange", b"endbfrange") {
        // Past the last `]` no array destination can close, so an entry
        // never searches the rest of the body again (linear time).
        let last_close = body.iter().rposition(|&b| b == b']');
        let mut at = 0;
        let mut consumed = 0;
        while at < body.len() {
            let Some((entry, end)) = bfrange_entry(body, at, last_close) else {
                at += 1;
                continue;
            };
            consumed = end;
            at = end;
            let (low_text, high_text, destination) = entry;
            if !is_hex(low_text, digits) || !is_hex(high_text, digits) {
                return None;
            }
            let (low, high) = (hex_value(low_text), hex_value(high_text));
            if high < low {
                return None;
            }
            let count = (high - low) as usize + 1;
            generated += count;
            if generated > MAX_CMAP_CODES {
                return None;
            }
            match destination {
                Destination::Hex(base) => {
                    if !is_hex(base, 4) {
                        continue;
                    }
                    let base = hex_value(base);
                    if base + (high - low) > 0xffff {
                        return None;
                    }
                    for code in low..=high {
                        if let Some(c) = char::from_u32(base + code - low) {
                            map.insert(code, c);
                        }
                    }
                }
                Destination::Array(items) => {
                    let destinations = hex_tokens(items);
                    for offset in 0..count {
                        let item = destinations.get(offset)?;
                        add(&mut map, low + offset as u32, item);
                    }
                }
            }
        }
        // A trailing fragment means this parser did not follow the block.
        let rest = &body[consumed.min(body.len())..];
        if (0..rest.len()).any(|i| hex_token(rest, i).is_some()) {
            return None;
        }
    }
    Some(map)
}

enum Destination<'t> {
    Hex(&'t [u8]),
    Array(&'t [u8]),
}

type RangeEntry<'t> = (&'t [u8], &'t [u8], Destination<'t>);

/// `<a>\s*<b>\s*(<c>|\[[\s\S]*?\])` at `at`; `last_close` is the body's
/// last `]`.
fn bfrange_entry(
    body: &[u8],
    at: usize,
    last_close: Option<usize>,
) -> Option<(RangeEntry<'_>, usize)> {
    let (low, end) = hex_token(body, at)?;
    let (high, end) = hex_token(body, skip_space(body, end))?;
    let start = skip_space(body, end);
    if let Some((base, end)) = hex_token(body, start) {
        return Some(((low, high, Destination::Hex(base)), end));
    }
    if body.get(start) != Some(&b'[') || last_close.is_none_or(|last| last < start) {
        return None;
    }
    let close = find(&body[start + 1..], b"]").map(|i| start + 1 + i)?;
    Some((
        (low, high, Destination::Array(&body[start + 1..close])),
        close + 1,
    ))
}

/// Every `<hex>` in `text`, in order.
fn hex_tokens(text: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        match hex_token(text, at) {
            Some((digits, end)) => {
                out.push(digits);
                at = end;
            }
            None => at += 1,
        }
    }
    out
}

/// content-edit.ts `editCidRepertoire`: the characters `codes` draw, in code
/// order; a character drawn by two codes is left out; at most 512.
pub fn repertoire(cid: &Cid, codes: &std::collections::BTreeSet<u32>) -> Option<Vec<(char, u32)>> {
    let mut first: Vec<(char, Option<u32>)> = Vec::new();
    for &code in codes {
        let Some(&c) = cid.to_unicode.get(&code) else {
            continue;
        };
        match first.iter_mut().find(|(seen, _)| *seen == c) {
            None => first.push((c, Some(code))),
            Some((_, slot)) => {
                if *slot != Some(code) {
                    *slot = None;
                }
            }
        }
    }
    let chars: Vec<(char, u32)> = first
        .into_iter()
        .filter_map(|(c, code)| Some((c, code?)))
        .take(512)
        .collect();
    (!chars.is_empty()).then_some(chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_unicode_forms() {
        let cmap = b"1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
            2 beginbfchar <0003> <0020> <0004> <D83DDE00> endbfchar\n\
            1 beginbfrange <0010> <0012> <0041> endbfrange\n\
            1 beginbfrange <0020> <0021> [<0061> <0062>] endbfrange";
        let map = parse_to_unicode(cmap, 2).unwrap();
        assert_eq!(map.get(&3), Some(&' '));
        assert_eq!(map.get(&4), None);
        assert_eq!(map.get(&0x11), Some(&'B'));
        assert_eq!(map.get(&0x21), Some(&'b'));
        assert!(parse_to_unicode(b"/usecmap", 2).is_none());
        assert!(parse_to_unicode(b"begincodespacerange <00> <FF> endcodespacerange", 2).is_none());
        let short = b"begincodespacerange <0000> <FFFF> endcodespacerange \
            beginbfrange <0001> <0003> [<0041>] endbfrange";
        assert!(parse_to_unicode(short, 2).is_none());
    }

    /// Review: an unclosed `[` searched the rest of the body once per
    /// entry (quadratic); now each search stops past the last `]`.
    #[test]
    fn an_unclosed_bfrange_array_is_linear() {
        let mut cmap =
            b"begincodespacerange <0000> <FFFF> endcodespacerange beginbfrange ".to_vec();
        for _ in 0..200_000 {
            cmap.extend_from_slice(b"<00><00>[");
        }
        cmap.extend_from_slice(b" endbfrange");
        assert!(parse_to_unicode(&cmap, 2).is_none());
        let mixed = b"begincodespacerange <0000> <FFFF> endcodespacerange \
            beginbfrange <0001> <0002> [<0041> <0042>] <0003> <0003> [<0043>] endbfrange";
        let map = parse_to_unicode(mixed, 2).unwrap();
        assert_eq!(map.get(&2), Some(&'B'));
        assert_eq!(map.get(&3), Some(&'C'));
    }

    #[test]
    fn win_ansi_round_trip() {
        for byte in 0..=255u8 {
            if let Some(c) = win_ansi_char(byte) {
                assert_eq!(win_ansi_byte(c), Some(byte));
            }
        }
        assert_eq!(win_ansi_byte('€'), Some(0x80));
        assert_eq!(win_ansi_char(0x81), None);
    }
}
