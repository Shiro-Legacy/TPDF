// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Unicode text of the character codes a page shows (ISO 32000-2
//! §9.10.2), for structured text: splitting shown strings into codes,
//! ToUnicode CMaps, simple-font encodings, glyph names by the Adobe Glyph
//! List specification, embedded TrueType `cmap` and `post` tables, and the
//! normalisation and bidirectional data structured text needs.
//!
//! The tables live in `text_unicode_data.rs`, generated from public
//! sources by `generate/generate_data.py`.
// Structured text (spec 04) is this module's caller; until it lands only
// the tests below use it.
#![cfg_attr(not(test), allow(dead_code))]

use crate::simple_encoding::{NamedEncoding, base_glyph_name};
use hayro_interpret::hayro_cmap::{
    BfString, CMap, CMapName, CidFamily, WritingMode, load_embedded,
};
use hayro_syntax::object::{Array, Dict, Name, Object, Stream};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

#[path = "text_unicode_data.rs"]
mod data;

/// Predefined CMaps parsed so far, by name, shared by the fonts of one
/// TextReader (a CJK collection's CMaps are large).
pub(super) type CMaps = HashMap<Vec<u8>, Option<Rc<CMap>>>;

/// One character code of shown text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Code {
    pub code: u32,
    /// The CID for a composite font; the code itself for a simple font.
    pub cid: u32,
    /// Bytes of the shown string the code took (1 to 4).
    pub bytes: u8,
}

const REPLACEMENT: char = '\u{FFFD}';

/// The most a ToUnicode or CMap stream, or a /CIDToGIDMap, decodes to.
const MAX_STREAM_BYTES: usize = 1 << 24;
/// `/Differences` items read; the rest are ignored.
const MAX_DIFFERENCES_ITEMS: usize = 1 << 16;
/// ToUnicode mappings kept (each a code, a range or an array); the rest are
/// ignored.
const MAX_TO_UNICODE_ENTRIES: usize = 1 << 20;
/// Bytes of one ToUnicode destination string (§9.10.3 allows 512).
const MAX_DESTINATION_BYTES: usize = 1024;
/// Character-to-glyph pairs read from a font's Unicode `cmap` to invert it.
const MAX_INVERTED_CHARS: usize = 0x11_0000;

/// The Unicode text of one font's character codes.
pub(super) struct FontText {
    to_unicode: ToUnicode,
    kind: Kind,
}

enum ToUnicode {
    Absent,
    /// `/ToUnicode /Identity-H` (or `-V`) on a Type0 font: each code is a
    /// UTF-16 code unit.
    Identity,
    Map(UnicodeMap),
}

enum Kind {
    /// The reading of each of the 256 codes when no ToUnicode entry maps
    /// it; `None`: the interpreter's reading.
    Simple(Vec<Option<Vec<char>>>),
    Composite(Composite),
}

struct Composite {
    /// The encoding CMap; `None` when it cannot be read.
    encoding: Option<Rc<CMap>>,
    /// The encoding is one of the predefined Unicode CMaps (§9.7.5.2,
    /// Table 116): its codes are UTF-16BE (or UCS-2) text.
    unicode: Option<UnicodeCodes>,
    vertical: bool,
    fallback: CidReading,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnicodeCodes {
    /// `Uni…-UCS2-…`: two-byte codes.
    Ucs2,
    /// `Uni…-UTF16-…`: two-byte codes, and four for a surrogate pair.
    Utf16,
}

impl UnicodeCodes {
    fn of(name: &[u8]) -> Option<Self> {
        let has = |part: &[u8]| name.windows(part.len()).any(|w| w == part);
        if !name.starts_with(b"Uni") {
            None
        } else if has(b"-UTF16-") {
            Some(Self::Utf16)
        } else if has(b"-UCS2-") {
            Some(Self::Ucs2)
        } else {
            None
        }
    }
}

/// How a composite font reads a CID without a ToUnicode entry (§9.10.2).
enum CidReading {
    /// The collection's `Adobe-<Ordering>-UCS2` CMap.
    Collection(Rc<CMap>),
    /// A CIDFontType2 without a program, Identity encoding: the CID is an
    /// index into the standard Macintosh glyph order.
    MacOrder,
    /// An embedded TrueType program, through its inverted Unicode `cmap`.
    Program(InvertedCmap),
    /// No information here: the interpreter's reading.
    Interpreter,
    Unknown,
}

impl FontText {
    /// `font` is a font dictionary (Type0 with its descendant, Type1,
    /// MMType1, TrueType or Type3); `program` the decoded embedded font
    /// program of the font (the descendant's for Type0), if any.
    pub(super) fn new(font: &Dict<'_>, program: Option<&[u8]>, cmaps: &mut CMaps) -> Self {
        let composite = name_of(font, b"Subtype").as_deref() == Some(b"Type0".as_slice());
        let to_unicode = read_to_unicode(font, composite, cmaps);
        let kind = if composite {
            Kind::Composite(Composite::new(font, program, cmaps))
        } else {
            Kind::Simple(simple_readings(font, program))
        };
        Self { to_unicode, kind }
    }

    /// Appends the codes of `shown`, split as the interpreter splits them
    /// (spec 05, 3.2).
    pub(super) fn codes(&self, shown: &[u8], out: &mut Vec<Code>) {
        match &self.kind {
            Kind::Simple(_) => out.extend(shown.iter().map(|&b| Code {
                code: u32::from(b),
                cid: u32::from(b),
                bytes: 1,
            })),
            Kind::Composite(font) => {
                let mut at = 0;
                while at < shown.len() {
                    let code = font.read_code(&shown[at..]);
                    at += usize::from(code.bytes);
                    out.push(code);
                }
            }
        }
    }

    /// The Unicode text of one code, or None: no information here, use the
    /// interpreter's reading of the glyph.
    pub(super) fn text(&self, code: Code) -> Option<Vec<char>> {
        let mapped = match &self.to_unicode {
            ToUnicode::Absent => None,
            ToUnicode::Identity => u16::try_from(code.code)
                .ok()
                .and_then(|unit| char::from_u32(u32::from(unit)))
                .map(|c| vec![c]),
            ToUnicode::Map(map) => map.lookup(code.code),
        };
        if let Some(chars) = mapped.map(controls).filter(|c| !c.is_empty()) {
            return Some(chars);
        }
        match &self.kind {
            Kind::Simple(readings) => readings
                .get(usize::try_from(code.code).ok()?)
                .cloned()
                .unwrap_or(Some(vec![REPLACEMENT])),
            Kind::Composite(font) => font.text(code),
        }
    }

    /// The font writes vertically (WMode 1 of its CMap, §9.7.4.3).
    pub(super) fn vertical(&self) -> bool {
        matches!(&self.kind, Kind::Composite(font) if font.vertical)
    }
}

/// The name value of `key` in `dict`.
fn name_of(dict: &Dict<'_>, key: &[u8]) -> Option<Vec<u8>> {
    dict.get::<Name<'_>>(key).map(|name| name.to_vec())
}

/// Applies the control rule (spec 05, 3.7): tab, LF, VT, FF and CR read as
/// spaces; other controls, NUL and the non-characters U+FFFE and U+FFFF
/// read as nothing.
fn controls(chars: impl IntoIterator<Item = char>) -> Vec<char> {
    chars
        .into_iter()
        .filter_map(|c| match c {
            '\u{9}'..='\u{D}' => Some(' '),
            '\u{0}'..='\u{1F}' | '\u{7F}'..='\u{9F}' | '\u{FFFE}' | '\u{FFFF}' => None,
            c => Some(c),
        })
        .collect()
}

/// The first source's text after the control rule, if any.
fn usable(chars: Option<Vec<char>>) -> Option<Vec<char>> {
    chars.map(controls).filter(|c| !c.is_empty())
}

fn bf_chars(value: BfString) -> Vec<char> {
    match value {
        BfString::Char(c) => vec![c],
        BfString::String(s) => s.chars().collect(),
    }
}

/// A predefined CMap by name, parsed once per `cmaps`.
fn predefined(name: &[u8], cmaps: &mut CMaps) -> Option<Rc<CMap>> {
    if let Some(found) = cmaps.get(name) {
        return found.clone();
    }
    let parsed = match CMapName::from_bytes(name) {
        CMapName::IdentityH => Some(CMap::identity_h()),
        CMapName::IdentityV => Some(CMap::identity_v()),
        other => load_embedded(other).and_then(|data| CMap::parse(data, load_embedded)),
    }
    .map(Rc::new);
    cmaps.insert(name.to_vec(), parsed.clone());
    parsed
}

fn decoded(stream: &Stream<'_>) -> Option<Vec<u8>> {
    stream
        .decoded_within(MAX_STREAM_BYTES)
        .ok()
        .map(|data| data.into_owned())
}

fn read_to_unicode(font: &Dict<'_>, composite: bool, cmaps: &mut CMaps) -> ToUnicode {
    match font.get::<Object<'_>>(b"ToUnicode") {
        Some(Object::Name(name))
            if composite && matches!(&*name, b"Identity-H" | b"Identity-V") =>
        {
            ToUnicode::Identity
        }
        Some(Object::Stream(stream)) => match decoded(&stream) {
            Some(data) => {
                let map = UnicodeMap::parse(&data, cmaps);
                if map.is_empty() {
                    ToUnicode::Absent
                } else {
                    ToUnicode::Map(map)
                }
            }
            None => ToUnicode::Absent,
        },
        _ => ToUnicode::Absent,
    }
}

// ------------------------------------------------------------------ composite fonts

impl Composite {
    fn new(font: &Dict<'_>, program: Option<&[u8]>, cmaps: &mut CMaps) -> Self {
        let mut identity = false;
        let mut unicode = None;
        let (encoding, vertical) = match font.get::<Object<'_>>(b"Encoding") {
            Some(Object::Name(name)) => {
                identity = matches!(&*name, b"Identity-H" | b"Identity-V");
                unicode = UnicodeCodes::of(&name);
                let cmap = predefined(&name, cmaps);
                let vertical = &*name == b"Identity-V" || writes_vertically(cmap.as_deref());
                (cmap, vertical)
            }
            Some(Object::Stream(stream)) => {
                let cmap = decoded(&stream)
                    .and_then(|data| CMap::parse(&data, load_embedded))
                    .map(Rc::new);
                // The stream dictionary's /WMode wins over the program's
                // (§9.7.5.3).
                let vertical = match stream.dict().get::<Object<'_>>(b"WMode") {
                    Some(Object::Number(mode)) => mode.as_f64() == 1.0,
                    _ => writes_vertically(cmap.as_deref()),
                };
                (cmap, vertical)
            }
            _ => (None, false),
        };
        let descendant = font
            .get::<Array<'_>>(b"DescendantFonts")
            .and_then(|fonts| fonts.iter::<Dict<'_>>().next())
            .unwrap_or_default();
        let fallback = cid_reading(&descendant, encoding.as_deref(), identity, program, cmaps);
        Self {
            encoding,
            unicode,
            vertical,
            fallback,
        }
    }

    /// The code at the start of `shown`, split as hayro-interpret splits it:
    /// the shortest prefix of 1 to 4 bytes the encoding CMap maps, else one
    /// byte read as code 0. A Unicode CMap's codes are split by their
    /// codespace (UTF-16 units and surrogate pairs), mapped to a CID or
    /// not.
    fn read_code(&self, shown: &[u8]) -> Code {
        let unit = |at: usize| u16_at(shown, at);
        let bytes = match (self.unicode, unit(0), unit(2)) {
            (Some(UnicodeCodes::Utf16), Some(0xD800..=0xDBFF), Some(0xDC00..=0xDFFF)) => 4,
            (Some(_), Some(_), _) => 2,
            _ => 0,
        };
        if bytes > 0 {
            let code = shown[..bytes]
                .iter()
                .fold(0u32, |code, &b| (code << 8) | u32::from(b));
            let cid = self
                .encoding
                .as_ref()
                .and_then(|cmap| cmap.lookup_cid_code(code, bytes as u8))
                .unwrap_or(0);
            return Code {
                code,
                cid,
                bytes: bytes as u8,
            };
        }
        if let Some(cmap) = &self.encoding {
            let mut code = 0u32;
            for (n, byte) in shown.iter().take(4).enumerate() {
                code = (code << 8) | u32::from(*byte);
                let bytes = (n + 1) as u8;
                if let Some(cid) = cmap.lookup_cid_code(code, bytes) {
                    return Code { code, cid, bytes };
                }
            }
        }
        Code {
            code: 0,
            cid: 0,
            bytes: 1,
        }
    }

    fn text(&self, code: Code) -> Option<Vec<char>> {
        if self.unicode.is_some() {
            let units = [(code.code >> 16) as u16, code.code as u16];
            let units = if code.bytes == 4 {
                &units[..]
            } else {
                &units[1..]
            };
            if let Some(chars) = usable(Some(utf16(units))) {
                return Some(chars);
            }
        }
        let cid = code.cid;
        let replacement = || vec![REPLACEMENT];
        match &self.fallback {
            CidReading::Collection(ucs2) => {
                Some(usable(ucs2.lookup_bf_string(cid).map(bf_chars)).unwrap_or_else(replacement))
            }
            CidReading::MacOrder => {
                let name = usize::try_from(cid)
                    .ok()
                    .and_then(|i| data::MAC_GLYPH_ORDER.get(i));
                Some(usable(name.and_then(|n| glyph_name(n, false))).unwrap_or_else(replacement))
            }
            CidReading::Program(inverted) => usable(inverted.lookup(cid).map(|c| vec![c])),
            CidReading::Interpreter => None,
            CidReading::Unknown => Some(replacement()),
        }
    }
}

fn writes_vertically(cmap: Option<&CMap>) -> bool {
    cmap.is_some_and(|cmap| cmap.metadata().writing_mode == Some(WritingMode::Vertical))
}

fn has_program(descriptor: Option<&Dict<'_>>) -> bool {
    descriptor.is_some_and(|d| {
        [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
            .iter()
            .any(|key| d.contains_key(key))
    })
}

fn cid_reading(
    descendant: &Dict<'_>,
    encoding: Option<&CMap>,
    identity: bool,
    program: Option<&[u8]>,
    cmaps: &mut CMaps,
) -> CidReading {
    let family = descendant
        .get::<Dict<'_>>(b"CIDSystemInfo")
        .and_then(|info| {
            let registry = info.get::<hayro_syntax::object::String<'_>>(b"Registry")?;
            let ordering = info.get::<hayro_syntax::object::String<'_>>(b"Ordering")?;
            Some(CidFamily::from_registry_ordering(
                registry.as_bytes(),
                ordering.as_bytes(),
            ))
        })
        .filter(|family| family.ucs2_cmap().is_some())
        .or_else(|| {
            let collection = encoding?.metadata().character_collection.as_ref()?;
            Some(collection.family.clone())
        });
    if let Some(name) = family.and_then(|family| family.ucs2_cmap()) {
        if let Some(ucs2) = predefined(name.to_bytes(), cmaps) {
            return CidReading::Collection(ucs2);
        }
    }
    if name_of(descendant, b"Subtype").as_deref() != Some(b"CIDFontType2".as_slice()) {
        return CidReading::Unknown;
    }
    let descriptor = descendant.get::<Dict<'_>>(b"FontDescriptor");
    if program.is_some() || has_program(descriptor.as_ref()) {
        return match program.and_then(|program| InvertedCmap::new(program, descendant)) {
            Some(inverted) => CidReading::Program(inverted),
            None => CidReading::Interpreter,
        };
    }
    if identity {
        CidReading::MacOrder
    } else {
        CidReading::Unknown
    }
}

/// An embedded TrueType program's Unicode `cmap`, inverted: the character of
/// each glyph that exactly one character maps to.
struct InvertedCmap {
    /// `/CIDToGIDMap` as a stream; `None`: Identity.
    cid_to_gid: Option<Vec<u16>>,
    /// Sorted by glyph.
    glyphs: Vec<(u16, char)>,
}

impl InvertedCmap {
    fn new(program: &[u8], descendant: &Dict<'_>) -> Option<Self> {
        let sfnt = Sfnt::parse(program)?;
        if sfnt.cff {
            return None;
        }
        let cmap = sfnt.table(b"cmap")?;
        let subtables = cmap_subtables(cmap);
        let unicode = [(3, 10), (3, 1)]
            .iter()
            .find_map(|&(p, e)| {
                subtables
                    .iter()
                    .find(|s| s.platform == p && s.encoding == e)
            })
            .or_else(|| subtables.iter().find(|s| s.platform == 0))?;
        let mut pairs = Vec::new();
        unicode.each_mapping(&mut |c, glyph| {
            if let Some(c) = char::from_u32(c) {
                if glyph != 0 {
                    pairs.push((glyph, c));
                }
            }
        });
        pairs.sort_unstable();
        pairs.dedup();
        let mut glyphs: Vec<(u16, char)> = Vec::with_capacity(pairs.len());
        let mut i = 0;
        while i < pairs.len() {
            let glyph = pairs[i].0;
            let run = pairs[i..].iter().take_while(|p| p.0 == glyph).count();
            if run == 1 {
                glyphs.push(pairs[i]);
            }
            i += run;
        }
        let cid_to_gid = descendant.get::<Stream<'_>>(b"CIDToGIDMap").map(|stream| {
            decoded(&stream)
                .unwrap_or_default()
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect()
        });
        Some(Self { cid_to_gid, glyphs })
    }

    fn lookup(&self, cid: u32) -> Option<char> {
        let glyph = match &self.cid_to_gid {
            Some(map) => *map.get(usize::try_from(cid).ok()?)?,
            None => u16::try_from(cid).ok()?,
        };
        let at = self.glyphs.binary_search_by_key(&glyph, |p| p.0).ok()?;
        self.glyphs.get(at).map(|p| p.1)
    }
}

// ------------------------------------------------------------------ simple fonts

/// Symbol and ZapfDingbats by name (spec 05, 3.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Standard {
    Symbol,
    ZapfDingbats,
    Other,
}

/// What a simple font's readings depend on.
struct SimpleInfo {
    type3: bool,
    standard: Standard,
    symbolic: bool,
    embedded: bool,
}

impl SimpleInfo {
    fn read(font: &Dict<'_>) -> Self {
        let type3 = name_of(font, b"Subtype").as_deref() == Some(b"Type3".as_slice());
        let standard =
            name_of(font, b"BaseFont").map_or(Standard::Other, |name| standard_of(&name));
        let descriptor = font.get::<Dict<'_>>(b"FontDescriptor");
        let flags = descriptor
            .as_ref()
            .and_then(|d| match d.get::<Object<'_>>(b"Flags") {
                Some(Object::Number(flags)) => Some(flags.as_i64()),
                _ => None,
            });
        let symbolic = match flags {
            Some(flags) => flags & 4 != 0 && flags & 32 == 0,
            None => standard != Standard::Other,
        };
        Self {
            type3,
            standard,
            symbolic,
            embedded: has_program(descriptor.as_ref()),
        }
    }

    /// The base encoding of a font without one of its own (spec 05, 3.4).
    fn default_base(&self) -> Option<NamedEncoding> {
        (!self.type3 && !self.embedded && !self.symbolic && self.standard == Standard::Other)
            .then_some(NamedEncoding::Standard)
    }
}

fn standard_of(base_font: &[u8]) -> Standard {
    let mut name = base_font;
    if name.len() > 7 && name[6] == b'+' && name[..6].iter().all(u8::is_ascii_uppercase) {
        name = &name[7..];
    }
    if let Some(comma) = name.iter().position(|&b| b == b',') {
        name = &name[..comma];
    }
    match name {
        b"Symbol" | b"SymbolMT" => Standard::Symbol,
        b"ZapfDingbats" | b"Dingbats" => Standard::ZapfDingbats,
        _ => Standard::Other,
    }
}

/// The glyph name of each code of a simple font's /Encoding (spec 05, 3.4):
/// base encoding, then /Differences. For Type3 fonts too.
pub(super) fn encoding_names(font: &Dict<'_>) -> Vec<Option<Vec<u8>>> {
    if name_of(font, b"Subtype").as_deref() == Some(b"Type0".as_slice()) {
        return vec![None; 256];
    }
    names_with(font, &SimpleInfo::read(font))
}

fn names_with(font: &Dict<'_>, info: &SimpleInfo) -> Vec<Option<Vec<u8>>> {
    let mut names = vec![None; 256];
    let named = |name: &Name<'_>| NamedEncoding::from_pdf_name(name);
    let (base, differences) = match font.get::<Object<'_>>(b"Encoding") {
        Some(Object::Name(name)) => (named(&name).or(info.default_base()), None),
        Some(Object::Dict(dict)) => (
            dict.get::<Name<'_>>(b"BaseEncoding")
                .and_then(|name| named(&name))
                .or(info.default_base()),
            dict.get::<Array<'_>>(b"Differences"),
        ),
        _ => (info.default_base(), None),
    };
    if let Some(base) = base {
        for (code, slot) in names.iter_mut().enumerate() {
            *slot = u8::try_from(code)
                .ok()
                .and_then(|code| base_glyph_name(base, code))
                .map(<[u8]>::to_vec);
        }
    }
    if let Some(differences) = differences {
        let mut cursor: Option<usize> = None;
        for item in differences.iter::<Object<'_>>().take(MAX_DIFFERENCES_ITEMS) {
            match item {
                Object::Number(number) => {
                    let value = number.as_f64();
                    // An out-of-range code, and the names after it, are
                    // skipped.
                    cursor = (value.fract() == 0.0 && (0.0..=255.0).contains(&value))
                        .then_some(value as usize);
                }
                Object::Name(name) => {
                    if let Some(code) = cursor {
                        if let Some(slot) = names.get_mut(code) {
                            *slot = Some(name.to_vec());
                        }
                        cursor = Some(code + 1);
                    }
                }
                _ => {}
            }
        }
    }
    names
}

/// The reading of each code of a simple font without a ToUnicode entry
/// (spec 05, 3.1 step 3).
fn simple_readings(font: &Dict<'_>, program: Option<&[u8]>) -> Vec<Option<Vec<char>>> {
    let info = SimpleInfo::read(font);
    let names = names_with(font, &info);
    let embedded = info.embedded || program.is_some();
    let truetype = program.and_then(|p| TrueTypeReader::new(p, info.symbolic));
    let dingbats = info.standard == Standard::ZapfDingbats;
    let mut out = Vec::with_capacity(256);
    for (code, name) in (0..=255u8).zip(&names) {
        let named = usable(name.as_deref().and_then(|n| glyph_name(n, dingbats)));
        let reading = if named.is_some() {
            named
        } else if info.type3 {
            Some(usable(type3_code_name(name.as_deref(), code)).unwrap_or(vec![REPLACEMENT]))
        } else if !embedded {
            let builtin = match info.standard {
                Standard::Symbol => builtin(&data::SYMBOL_BUILTIN, code),
                Standard::ZapfDingbats => builtin(&data::DINGBATS_BUILTIN, code),
                Standard::Other => base_glyph_name(NamedEncoding::Standard, code)
                    .and_then(|n| glyph_name(n, false)),
            };
            Some(usable(builtin).unwrap_or(vec![REPLACEMENT]))
        } else {
            truetype
                .as_ref()
                .map(|font| font.reading(code).unwrap_or(vec![REPLACEMENT]))
        };
        out.push(reading);
    }
    out
}

fn builtin(table: &[(u8, char)], code: u8) -> Option<Vec<char>> {
    let at = table.binary_search_by_key(&code, |e| e.0).ok()?;
    table.get(at).map(|e| vec![e.1])
}

/// A Type3 glyph named `a` and its own code in decimal reads as that code's
/// Latin-1 character (spec 05, 4.2).
fn type3_code_name(name: Option<&[u8]>, code: u8) -> Option<Vec<char>> {
    let digits = name?.strip_prefix(b"a")?;
    (digits == code.to_string().as_bytes()).then(|| vec![char::from(code)])
}

// ------------------------------------------------------------------ glyph names

/// A glyph name's Unicode text by the AGL specification (spec 05, 4.1), or
/// None. `dingbats`: the font is ZapfDingbats (use the ITC Zapf Dingbats
/// list).
pub(super) fn glyph_name(name: &[u8], dingbats: bool) -> Option<Vec<char>> {
    let base = match name.iter().position(|&b| b == b'.') {
        Some(dot) => &name[..dot],
        None => name,
    };
    if base.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for component in base.split(|&b| b == b'_') {
        out.extend_from_slice(&glyph_component(component, dingbats)?);
    }
    Some(out)
}

fn glyph_component(component: &[u8], dingbats: bool) -> Option<Vec<char>> {
    let listed = |list: &[(&[u8], &[char])]| {
        let at = list.binary_search_by(|e| e.0.cmp(component)).ok()?;
        list.get(at).map(|e| e.1.to_vec())
    };
    if dingbats {
        if let Some(chars) = listed(&data::ZAPF_DINGBATS_LIST) {
            return Some(chars);
        }
    }
    if let Some(chars) = listed(&data::AGL) {
        return Some(chars);
    }
    if let Some(digits) = component.strip_prefix(b"uni") {
        if digits.is_empty() || digits.len() % 4 != 0 {
            return None;
        }
        return digits
            .chunks(4)
            .map(|group| hex_value(group).and_then(scalar))
            .collect();
    }
    if let Some(digits) = component.strip_prefix(b"u") {
        if (4..=6).contains(&digits.len()) {
            return hex_value(digits).and_then(scalar).map(|c| vec![c]);
        }
    }
    None
}

/// Hex digits (either case) as a number.
fn hex_value(digits: &[u8]) -> Option<u32> {
    digits.iter().try_fold(0u32, |value, &digit| {
        let digit = char::from(digit).to_digit(16)?;
        value.checked_mul(16)?.checked_add(digit)
    })
}

/// A Unicode scalar value (no surrogates, at most U+10FFFF).
fn scalar(value: u32) -> Option<char> {
    char::from_u32(value)
}

// ------------------------------------------------------------------ normalisation and bidi

/// Appends the characters structured text gives `c` (spec 05, 4.3):
/// ligatures and presentation forms as their components, U+FEFF as nothing,
/// every other character as itself.
pub(super) fn normalise_extracted(c: char, out: &mut Vec<char>) {
    if c == '\u{FEFF}' {
        return;
    }
    match data::PRESENTATION_FORMS.binary_search_by_key(&c, |e| e.0) {
        Ok(at) => out.extend_from_slice(data::PRESENTATION_FORMS.get(at).map_or(&[c][..], |e| e.1)),
        Err(_) => out.push(c),
    }
}

/// The 23 bidirectional classes of UAX #9.
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BidiClass {
    L,
    R,
    AL,
    EN,
    ES,
    ET,
    AN,
    CS,
    NSM,
    BN,
    B,
    S,
    WS,
    ON,
    LRE,
    LRO,
    RLE,
    RLO,
    PDF,
    LRI,
    RLI,
    FSI,
    PDI,
}

/// Bidi_Class of `c` (UCD DerivedBidiClass.txt, defaults included): the
/// `unicode-bidi` crate's tables, which read the unassigned code points
/// whose default class is BN as L, with those code points from the
/// generated table.
pub(super) fn bidi_class(c: char) -> BidiClass {
    use unicode_bidi::BidiClass as U;
    let at = data::BIDI_DEFAULT_BN.partition_point(|range| range.1 < c);
    if data::BIDI_DEFAULT_BN
        .get(at)
        .is_some_and(|range| range.0 <= c)
    {
        return BidiClass::BN;
    }
    match unicode_bidi::bidi_class(c) {
        U::L => BidiClass::L,
        U::R => BidiClass::R,
        U::AL => BidiClass::AL,
        U::EN => BidiClass::EN,
        U::ES => BidiClass::ES,
        U::ET => BidiClass::ET,
        U::AN => BidiClass::AN,
        U::CS => BidiClass::CS,
        U::NSM => BidiClass::NSM,
        U::BN => BidiClass::BN,
        U::B => BidiClass::B,
        U::S => BidiClass::S,
        U::WS => BidiClass::WS,
        U::ON => BidiClass::ON,
        U::LRE => BidiClass::LRE,
        U::LRO => BidiClass::LRO,
        U::RLE => BidiClass::RLE,
        U::RLO => BidiClass::RLO,
        U::PDF => BidiClass::PDF,
        U::LRI => BidiClass::LRI,
        U::RLI => BidiClass::RLI,
        U::FSI => BidiClass::FSI,
        U::PDI => BidiClass::PDI,
    }
}

/// Bidi_Mirroring_Glyph of `c` (BidiMirroring.txt), if any.
pub(super) fn bidi_mirror(c: char) -> Option<char> {
    let at = data::MIRRORS.binary_search_by_key(&c, |e| e.0).ok()?;
    data::MIRRORS.get(at).map(|e| e.1)
}

// ------------------------------------------------------------------ ToUnicode CMaps

/// A ToUnicode CMap (§9.10.3), read tolerantly: a malformed entry is
/// skipped, the others map.
struct UnicodeMap {
    entries: Vec<Entry>,
    /// Disjoint code intervals: start → (end, entry). A later entry for a
    /// code replaces an earlier one.
    intervals: BTreeMap<u32, (u32, usize)>,
    /// A predefined CMap named by `usecmap`.
    base: Option<Rc<CMap>>,
}

struct Entry {
    first: u32,
    destination: Destination,
}

enum Destination {
    /// UTF-16 units; the n-th code of a range adds n to the last unit.
    Units(Vec<u16>),
    /// One destination per code of the range (`None`: malformed item).
    Array(Vec<Option<Vec<u16>>>),
    /// `cidchar`/`cidrange`: the CID read as a Unicode scalar.
    Cid(u32),
}

impl UnicodeMap {
    fn parse(data: &[u8], cmaps: &mut CMaps) -> Self {
        let mut map = Self {
            entries: Vec::new(),
            intervals: BTreeMap::new(),
            base: None,
        };
        let mut lexer = Lexer { data, at: 0 };
        let mut previous_name: Option<&[u8]> = None;
        while let Some(token) = lexer.next() {
            match token {
                Token::Word(b"beginbfchar") => map.bfchar(&mut lexer),
                Token::Word(b"beginbfrange") => map.bfrange(&mut lexer),
                Token::Word(b"begincidchar") => map.cidchar(&mut lexer),
                Token::Word(b"begincidrange") => map.cidrange(&mut lexer),
                Token::Word(b"usecmap") => {
                    if let Some(name) = previous_name {
                        map.base = predefined(name, cmaps);
                    }
                }
                _ => {}
            }
            previous_name = match token {
                Token::Name(name) => Some(name),
                _ => None,
            };
        }
        map
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.base.is_none()
    }

    fn bfchar(&mut self, lexer: &mut Lexer<'_>) {
        while let Some(source) = lexer.operand() {
            let Some(destination) = lexer.operand() else {
                return;
            };
            if let (Token::Hex(Some(code)), Token::Hex(Some(destination))) = (source, destination) {
                if let Some(code) = code_value(&code) {
                    self.add(code, code, Destination::Units(units(&destination)));
                }
            }
        }
    }

    fn bfrange(&mut self, lexer: &mut Lexer<'_>) {
        while let Some(first) = lexer.operand() {
            let Some(last) = lexer.operand() else {
                return;
            };
            let Some(destination) = lexer.operand() else {
                return;
            };
            let destination = match destination {
                Token::Hex(Some(bytes)) => Some(Destination::Units(units(&bytes))),
                Token::Open => {
                    let mut items = Vec::new();
                    loop {
                        match lexer.operand() {
                            Some(Token::Close) => break,
                            Some(Token::Hex(Some(bytes))) => items.push(Some(units(&bytes))),
                            Some(_) => items.push(None),
                            None => return,
                        }
                    }
                    Some(Destination::Array(items))
                }
                _ => None,
            };
            if let (Token::Hex(Some(first)), Token::Hex(Some(last)), Some(destination)) =
                (first, last, destination)
            {
                if let (Some(first), Some(last)) = (code_value(&first), code_value(&last)) {
                    if first <= last {
                        self.add(first, last, destination);
                    }
                }
            }
        }
    }

    fn cidchar(&mut self, lexer: &mut Lexer<'_>) {
        while let Some(source) = lexer.operand() {
            let Some(cid) = lexer.operand() else {
                return;
            };
            if let (Token::Hex(Some(code)), Some(cid)) = (source, cid.integer()) {
                if let Some(code) = code_value(&code) {
                    self.add(code, code, Destination::Cid(cid));
                }
            }
        }
    }

    fn cidrange(&mut self, lexer: &mut Lexer<'_>) {
        while let Some(first) = lexer.operand() {
            let Some(last) = lexer.operand() else {
                return;
            };
            let Some(cid) = lexer.operand() else {
                return;
            };
            if let (Token::Hex(Some(first)), Token::Hex(Some(last)), Some(cid)) =
                (first, last, cid.integer())
            {
                if let (Some(first), Some(last)) = (code_value(&first), code_value(&last)) {
                    if first <= last {
                        self.add(first, last, Destination::Cid(cid));
                    }
                }
            }
        }
    }

    /// Maps the codes `first..=last`, replacing earlier mappings of them.
    fn add(&mut self, first: u32, last: u32, destination: Destination) {
        if self.entries.len() >= MAX_TO_UNICODE_ENTRIES {
            return;
        }
        let index = self.entries.len();
        self.entries.push(Entry { first, destination });
        let mut kept = Vec::new();
        if let Some((&start, &(end, entry))) = self.intervals.range(..first).next_back() {
            if end >= first {
                self.intervals.remove(&start);
                kept.push((start, first - 1, entry));
                if end > last {
                    kept.push((last + 1, end, entry));
                }
            }
        }
        let inside: Vec<u32> = self
            .intervals
            .range(first..=last)
            .map(|(&s, _)| s)
            .collect();
        for start in inside {
            if let Some((end, entry)) = self.intervals.remove(&start) {
                if end > last {
                    kept.push((last + 1, end, entry));
                }
            }
        }
        for (start, end, entry) in kept {
            self.intervals.insert(start, (end, entry));
        }
        self.intervals.insert(first, (last, index));
    }

    /// The characters `code` maps to, before the control rule; `None`:
    /// unmapped.
    fn lookup(&self, code: u32) -> Option<Vec<char>> {
        let own = self
            .intervals
            .range(..=code)
            .next_back()
            .filter(|(_, (end, _))| *end >= code)
            .and_then(|(_, &(_, index))| self.entries.get(index))
            .and_then(|entry| entry.text(code));
        if own.is_some() {
            return own;
        }
        let base = self.base.as_ref()?;
        if let Some(value) = base.lookup_bf_string(code) {
            return Some(bf_chars(value));
        }
        (1..=4)
            .find_map(|bytes| base.lookup_cid_code(code, bytes))
            .and_then(char::from_u32)
            .map(|c| vec![c])
    }
}

impl Entry {
    fn text(&self, code: u32) -> Option<Vec<char>> {
        let offset = code.checked_sub(self.first)?;
        match &self.destination {
            Destination::Units(units) => {
                let mut units = units.clone();
                if let Some(last) = units.last_mut() {
                    *last = u16::try_from(u32::from(*last).checked_add(offset)?).ok()?;
                }
                Some(utf16(&units))
            }
            Destination::Array(items) => {
                let units = items.get(usize::try_from(offset).ok()?)?.as_ref()?;
                Some(utf16(units))
            }
            Destination::Cid(cid) => Some(vec![char::from_u32(cid.checked_add(offset)?)?]),
        }
    }
}

/// UTF-16 units as characters, surrogate pairs joined; unpaired surrogates
/// read as nothing.
fn utf16(units: &[u16]) -> Vec<char> {
    char::decode_utf16(units.iter().copied())
        .filter_map(Result::ok)
        .collect()
}

/// A destination string's UTF-16 units: one byte reads as Latin-1; another
/// odd length is padded with a zero byte.
fn units(bytes: &[u8]) -> Vec<u16> {
    if let [byte] = bytes {
        return vec![u16::from(*byte)];
    }
    bytes
        .chunks(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair.get(1).copied().unwrap_or(0)]))
        .collect()
}

/// A source code of 1 to 4 bytes as a number.
fn code_value(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || bytes.len() > 4 {
        return None;
    }
    Some(bytes.iter().fold(0, |code, &b| (code << 8) | u32::from(b)))
}

#[derive(Debug, PartialEq)]
enum Token<'a> {
    /// A hex string's bytes; `None`: malformed.
    Hex(Option<Vec<u8>>),
    Name(&'a [u8]),
    Word(&'a [u8]),
    Open,
    Close,
    /// Anything else: literal strings, dictionary brackets, stray
    /// delimiters.
    Other,
}

impl Token<'_> {
    fn integer(&self) -> Option<u32> {
        match self {
            Token::Word(word) if !word.is_empty() && word.iter().all(u8::is_ascii_digit) => {
                std::str::from_utf8(word).ok()?.parse().ok()
            }
            _ => None,
        }
    }
}

/// The tokens of a CMap program (PostScript syntax, as far as CMaps use
/// it).
struct Lexer<'a> {
    data: &'a [u8],
    at: usize,
}

fn is_white(b: u8) -> bool {
    matches!(b, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<u8> {
        self.data.get(self.at).copied()
    }

    fn next(&mut self) -> Option<Token<'a>> {
        loop {
            let b = self.peek()?;
            if is_white(b) {
                self.at += 1;
            } else if b == b'%' {
                while let Some(b) = self.peek() {
                    if b == b'\n' || b == b'\r' {
                        break;
                    }
                    self.at += 1;
                }
            } else {
                break;
            }
        }
        let b = self.peek()?;
        self.at += 1;
        Some(match b {
            b'(' => {
                self.skip_string();
                Token::Other
            }
            b'<' if self.peek() == Some(b'<') => {
                self.at += 1;
                Token::Other
            }
            b'<' => Token::Hex(self.hex()),
            b'>' => {
                if self.peek() == Some(b'>') {
                    self.at += 1;
                }
                Token::Other
            }
            b'[' => Token::Open,
            b']' => Token::Close,
            b'/' => Token::Name(self.regular()),
            _ if is_delimiter(b) => Token::Other,
            _ => {
                self.at -= 1;
                Token::Word(self.regular())
            }
        })
    }

    /// The next operand of a mapping block, or `None` at the block's end
    /// (an `end…` keyword) or the end of data. A `begin…` keyword also ends
    /// the block and is handed back, so the next block is still read.
    fn operand(&mut self) -> Option<Token<'a>> {
        let before = self.at;
        match self.next()? {
            Token::Word(word) if word.starts_with(b"end") => None,
            Token::Word(word) if word.starts_with(b"begin") => {
                self.at = before;
                None
            }
            token => Some(token),
        }
    }

    fn regular(&mut self) -> &'a [u8] {
        let start = self.at;
        while let Some(b) = self.peek() {
            if is_white(b) || is_delimiter(b) {
                break;
            }
            self.at += 1;
        }
        &self.data[start..self.at]
    }

    fn skip_string(&mut self) {
        let mut depth = 1usize;
        while let Some(b) = self.peek() {
            self.at += 1;
            match b {
                b'\\' => self.at += 1,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// The bytes of a hex string after its `<`, through its `>`.
    fn hex(&mut self) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut high: Option<u8> = None;
        let mut good = true;
        loop {
            let b = self.peek()?;
            self.at += 1;
            if b == b'>' {
                break;
            }
            if is_white(b) {
                continue;
            }
            match char::from(b).to_digit(16) {
                Some(digit) if bytes.len() < MAX_DESTINATION_BYTES => {
                    let digit = digit as u8;
                    match high.take() {
                        Some(h) => bytes.push(h << 4 | digit),
                        None => high = Some(digit),
                    }
                }
                _ => good = false,
            }
        }
        if let Some(h) = high {
            bytes.push(h << 4);
        }
        good.then_some(bytes)
    }
}

// ------------------------------------------------------------------ TrueType programs

/// An sfnt font program's table directory.
struct Sfnt<'a> {
    tables: Vec<([u8; 4], &'a [u8])>,
    /// CFF outlines (`OTTO`, or a `CFF ` table).
    cff: bool,
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    let bytes = data.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

impl<'a> Sfnt<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let mut start = 0usize;
        if data.get(0..4)? == b"ttcf" {
            start = usize::try_from(u32_at(data, 12)?).ok()?;
        }
        let version = data.get(start..start.checked_add(4)?)?;
        let otto = version == b"OTTO";
        if !(otto || version == [0, 1, 0, 0] || version == b"true") {
            return None;
        }
        let count = usize::from(u16_at(data, start + 4)?);
        let mut tables = Vec::new();
        for i in 0..count {
            let record = start + 12 + 16 * i;
            let Some(tag) = data.get(record..record + 4) else {
                break;
            };
            let (Some(offset), Some(length)) =
                (u32_at(data, record + 8), u32_at(data, record + 12))
            else {
                break;
            };
            let offset = usize::try_from(offset).ok()?;
            let end = offset
                .saturating_add(usize::try_from(length).ok()?)
                .min(data.len());
            let table = data.get(offset..end).unwrap_or_default();
            let mut name = [0u8; 4];
            name.copy_from_slice(tag);
            tables.push((name, table));
        }
        let cff = otto
            || tables
                .iter()
                .any(|(tag, _)| tag == b"CFF " || tag == b"CFF2");
        Some(Self { tables, cff })
    }

    fn table(&self, tag: &[u8; 4]) -> Option<&'a [u8]> {
        self.tables
            .iter()
            .find(|(t, _)| t == tag)
            .map(|(_, data)| *data)
    }
}

/// One `cmap` subtable of a supported format (0, 4, 6, 12, 13).
struct Subtable<'a> {
    platform: u16,
    encoding: u16,
    format: u16,
    /// From the subtable's start to the end of the `cmap` table: a table
    /// that claims more than this reads what is there.
    data: &'a [u8],
}

fn cmap_subtables(cmap: &[u8]) -> Vec<Subtable<'_>> {
    let count = u16_at(cmap, 2).unwrap_or(0);
    let mut out = Vec::new();
    for i in 0..usize::from(count) {
        let record = 4 + 8 * i;
        let (Some(platform), Some(encoding), Some(offset)) = (
            u16_at(cmap, record),
            u16_at(cmap, record + 2),
            u32_at(cmap, record + 4),
        ) else {
            break;
        };
        let Some(data) = usize::try_from(offset).ok().and_then(|o| cmap.get(o..)) else {
            continue;
        };
        let Some(format) = u16_at(data, 0) else {
            continue;
        };
        if matches!(format, 0 | 4 | 6 | 12 | 13) {
            out.push(Subtable {
                platform,
                encoding,
                format,
                data,
            });
        }
    }
    out
}

impl Subtable<'_> {
    /// The glyph of character code `c` (0: none).
    fn glyph(&self, c: u32) -> u16 {
        let data = self.data;
        let glyph = match self.format {
            0 => usize::try_from(c)
                .ok()
                .filter(|&c| c < 256)
                .and_then(|c| data.get(6 + c))
                .map(|&g| u16::from(g)),
            4 => u16::try_from(c).ok().and_then(|c| self.format4(c)),
            6 => (|| {
                let first = u32::from(u16_at(data, 6)?);
                let count = u32::from(u16_at(data, 8)?);
                let index = c.checked_sub(first).filter(|&i| i < count)?;
                u16_at(data, 10 + 2 * usize::try_from(index).ok()?)
            })(),
            12 | 13 => self.group(c).and_then(|(start, _, glyph)| {
                let glyph = if self.format == 12 {
                    glyph.checked_add(c - start)?
                } else {
                    glyph
                };
                u16::try_from(glyph).ok()
            }),
            _ => None,
        };
        glyph.unwrap_or(0)
    }

    /// Format 4: the number of segments.
    fn segments(&self) -> usize {
        u16_at(self.data, 6).map_or(0, |n| usize::from(n / 2))
    }

    /// Format 4: segment `i`'s end code, or `None` past the bytes.
    fn segment_end(&self, i: usize) -> Option<u16> {
        u16_at(self.data, 14 + 2 * i)
    }

    /// Format 4: the glyph segment `i` gives `c`, which lies at or before
    /// the segment's end (0: none).
    fn segment_glyph(&self, i: usize, c: u16) -> Option<u16> {
        let data = self.data;
        let segments = self.segments();
        let starts = 16 + 2 * segments;
        let deltas = starts + 2 * segments;
        let offsets = deltas + 2 * segments;
        let start = u16_at(data, starts + 2 * i)?;
        if start > c {
            return Some(0);
        }
        let delta = u16_at(data, deltas + 2 * i)?;
        let range_offset = u16_at(data, offsets + 2 * i)?;
        if range_offset == 0 {
            return Some(c.wrapping_add(delta));
        }
        let at = offsets + 2 * i + usize::from(range_offset) + 2 * usize::from(c - start);
        let glyph = u16_at(data, at)?;
        Some(if glyph == 0 {
            0
        } else {
            glyph.wrapping_add(delta)
        })
    }

    /// Format 4: the first segment, in table order, whose end is at or
    /// after `c` decides.
    fn format4(&self, c: u16) -> Option<u16> {
        for i in 0..self.segments() {
            if self.segment_end(i)? >= c {
                return self.segment_glyph(i, c);
            }
        }
        None
    }

    /// The number of groups of a format 12 or 13 subtable: as claimed, at
    /// most as many as its bytes hold.
    fn groups(&self) -> usize {
        let claimed = u32_at(self.data, 12).map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
        claimed.min(self.data.len().saturating_sub(16) / 12)
    }

    fn group_at(&self, i: usize) -> Option<(u32, u32, u32)> {
        let at = 16 + 12 * i;
        Some((
            u32_at(self.data, at)?,
            u32_at(self.data, at + 4)?,
            u32_at(self.data, at + 8)?,
        ))
    }

    /// Formats 12 and 13: the group holding `c`, by bisection (groups are
    /// sorted by their first code).
    fn group(&self, c: u32) -> Option<(u32, u32, u32)> {
        let count = self.groups();
        let (mut low, mut high) = (0usize, count);
        while low < high {
            let mid = low + (high - low) / 2;
            let (start, _, _) = self.group_at(mid)?;
            if start <= c {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        let group = self.group_at(low.checked_sub(1)?)?;
        (group.0 <= c && c <= group.1).then_some(group)
    }

    /// Calls `f(code, glyph)` for the codes the subtable maps, each once,
    /// at most `MAX_INVERTED_CHARS` in all.
    fn each_mapping(&self, f: &mut dyn FnMut(u32, u16)) {
        let mut budget = MAX_INVERTED_CHARS;
        match self.format {
            0 | 6 => {
                let (first, last) = if self.format == 0 {
                    (0, 255)
                } else {
                    match (u16_at(self.data, 6), u16_at(self.data, 8)) {
                        (Some(first), Some(count)) if count > 0 => {
                            (u32::from(first), u32::from(first) + u32::from(count) - 1)
                        }
                        _ => return,
                    }
                };
                for c in first..=last.min(0xFFFF) {
                    let glyph = self.glyph(c);
                    if glyph != 0 {
                        f(c, glyph);
                    }
                }
            }
            4 => {
                // One pass over the segments in table order: the codes
                // after those already decided, up to a segment's end, are
                // that segment's. Each code is visited at most once.
                let mut decided: u32 = 0;
                for i in 0..self.segments() {
                    let Some(end) = self.segment_end(i) else {
                        return;
                    };
                    for c in decided..=u32::from(end) {
                        let Ok(c) = u16::try_from(c) else {
                            return;
                        };
                        if let Some(glyph) = self.segment_glyph(i, c).filter(|&g| g != 0) {
                            f(u32::from(c), glyph);
                        }
                    }
                    decided = decided.max(u32::from(end) + 1);
                    if decided > 0xFFFF {
                        return;
                    }
                }
            }
            12 | 13 => {
                let count = self.groups();
                for i in 0..count {
                    let Some((start, end, glyph)) = self.group_at(i) else {
                        return;
                    };
                    for c in start..=end.min(0x10_FFFF) {
                        if budget == 0 {
                            return;
                        }
                        budget -= 1;
                        let glyph = if self.format == 12 {
                            glyph.checked_add(c - start)
                        } else {
                            Some(glyph)
                        };
                        if let Some(glyph) = glyph.and_then(|g| u16::try_from(g).ok()) {
                            f(c, glyph);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Glyph names from a `post` table (formats 1, 2 and 2.5).
enum PostNames<'a> {
    None,
    Standard,
    /// Format 2: each glyph's index into the standard names, then the
    /// font's own names.
    Indexed {
        indexes: &'a [u8],
        names: Vec<&'a [u8]>,
    },
    /// Format 2.5: each glyph's offset into the standard order.
    Offsets(&'a [u8]),
}

impl<'a> PostNames<'a> {
    fn new(post: Option<&'a [u8]>) -> Self {
        let Some(post) = post else {
            return Self::None;
        };
        match u32_at(post, 0) {
            Some(0x0001_0000) => Self::Standard,
            Some(0x0002_0000) => {
                let count = usize::from(u16_at(post, 32).unwrap_or(0));
                let end = (34 + 2 * count).min(post.len());
                let indexes = post.get(34..end).unwrap_or_default();
                let mut names = Vec::new();
                let mut at = end;
                while let Some(&len) = post.get(at) {
                    let Some(name) = post.get(at + 1..at + 1 + usize::from(len)) else {
                        break;
                    };
                    names.push(name);
                    at += 1 + usize::from(len);
                }
                Self::Indexed { indexes, names }
            }
            Some(0x0002_5000) => {
                let count = usize::from(u16_at(post, 32).unwrap_or(0));
                let end = (34 + count).min(post.len());
                Self::Offsets(post.get(34..end).unwrap_or_default())
            }
            _ => Self::None,
        }
    }

    fn name(&self, glyph: u16) -> Option<&'a [u8]> {
        let glyph = usize::from(glyph);
        match self {
            Self::None => None,
            Self::Standard => data::MAC_GLYPH_ORDER.get(glyph).copied(),
            Self::Indexed { indexes, names } => {
                let index = usize::from(u16_at(indexes, 2 * glyph)?);
                match index.checked_sub(data::MAC_GLYPH_ORDER.len()) {
                    None => data::MAC_GLYPH_ORDER.get(index).copied(),
                    Some(own) => names.get(own).copied(),
                }
            }
            Self::Offsets(offsets) => {
                let offset = i8::from_be_bytes([*offsets.get(glyph)?]);
                let index = glyph.checked_add_signed(isize::from(offset))?;
                data::MAC_GLYPH_ORDER.get(index).copied()
            }
        }
    }
}

/// An embedded TrueType program's reading of a simple font's codes
/// (spec 05, 3.5).
struct TrueTypeReader<'a> {
    subtable: Option<Subtable<'a>>,
    names: PostNames<'a>,
    symbolic: bool,
}

impl<'a> TrueTypeReader<'a> {
    fn new(program: &'a [u8], symbolic: bool) -> Option<Self> {
        let sfnt = Sfnt::parse(program)?;
        if sfnt.cff {
            return None;
        }
        let subtables = sfnt.table(b"cmap").map(cmap_subtables).unwrap_or_default();
        let find = |p: u16, e: u16| {
            subtables
                .iter()
                .position(|s| s.platform == p && s.encoding == e)
        };
        let chosen = (if symbolic { find(3, 0) } else { None })
            .or_else(|| find(3, 1))
            .or_else(|| find(1, 0))
            .or(if subtables.is_empty() { None } else { Some(0) });
        let subtable = chosen.and_then(|i| subtables.into_iter().nth(i));
        Some(Self {
            subtable,
            names: PostNames::new(sfnt.table(b"post")),
            symbolic,
        })
    }

    fn glyph(&self, code: u8) -> u16 {
        let Some(subtable) = &self.subtable else {
            return 0;
        };
        let c = u32::from(code);
        if (subtable.platform, subtable.encoding) == (3, 0) {
            [c, 0xF000 + c, 0xF100 + c, 0xF200 + c]
                .into_iter()
                .map(|c| subtable.glyph(c))
                .find(|&g| g != 0)
                .unwrap_or(0)
        } else {
            subtable.glyph(c)
        }
    }

    /// The text of `code`, or `None` when the program does not say.
    fn reading(&self, code: u8) -> Option<Vec<char>> {
        let glyph = self.glyph(code);
        if glyph == 0 {
            return None;
        }
        let named = usable(self.names.name(glyph).and_then(|n| glyph_name(n, false)));
        if named.is_some() {
            return named;
        }
        // A symbolic font's glyph without a usable name, shown by a
        // printable ASCII code: writers mark subset fonts symbolic while
        // using ASCII codes, so the code reads as WinAnsi (= ASCII here).
        (self.symbolic && (0x20..=0x7E).contains(&code)).then(|| vec![char::from(code)])
    }
}

#[cfg(test)]
mod tests {
    //! The acceptance cases of spec 05 (section 8.2) at the level of this
    //! module: each builds its font in a small PDF and reads its codes.
    use super::*;
    use hayro_syntax::Pdf;
    use hayro_syntax::object::ObjectIdentifier;
    use std::time::{Duration, Instant};

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/adobe-symbol-encodings.rs"
    ));

    const FFFD: &str = "\u{fffd}";

    /// A classic-xref PDF: a catalog and an empty page tree, then `objects`
    /// numbered from 3.
    fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
        let mut all = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [] /Count 0 >>".to_vec(),
        ];
        all.extend(objects.iter().cloned());
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in all.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n", i + 1).bytes());
            out.extend(body);
            out.extend(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", all.len() + 1).bytes());
        for offset in offsets {
            out.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                all.len() + 1
            )
            .bytes(),
        );
        out
    }

    fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        out.extend(data);
        out.extend(b"\nendstream");
        out
    }

    /// The text of each code of `shown` in the font that is object 3 of
    /// `objects`, normalised as structured text does; `<none>` where this
    /// module leaves the reading to the interpreter.
    fn read_codes(objects: &[Vec<u8>], program: Option<&[u8]>, shown: &[u8]) -> Vec<String> {
        let pdf = Pdf::new(pdf(objects)).unwrap();
        let font = pdf
            .xref()
            .get::<Dict<'_>>(ObjectIdentifier::new(3, 0))
            .unwrap();
        let text = FontText::new(&font, program, &mut CMaps::new());
        let mut codes = Vec::new();
        text.codes(shown, &mut codes);
        codes
            .iter()
            .map(|&code| match text.text(code) {
                Some(chars) => {
                    let mut out = Vec::new();
                    for c in chars {
                        normalise_extracted(c, &mut out);
                    }
                    out.into_iter().collect()
                }
                None => "<none>".to_string(),
            })
            .collect()
    }

    fn read(objects: &[Vec<u8>], program: Option<&[u8]>, shown: &[u8]) -> String {
        read_codes(objects, program, shown).concat()
    }

    fn font_of(objects: &[Vec<u8>], program: Option<&[u8]>) -> FontText {
        let pdf = Pdf::new(pdf(objects)).unwrap();
        let font = pdf
            .xref()
            .get::<Dict<'_>>(ObjectIdentifier::new(3, 0))
            .unwrap();
        FontText::new(&font, program, &mut CMaps::new())
    }

    fn u(values: &[u32]) -> String {
        values.iter().map(|&v| char::from_u32(v).unwrap()).collect()
    }

    // ------------------------------------------------------------ N1, N10 glyph names

    fn type3(codes: &[(u8, &str)]) -> Vec<Vec<u8>> {
        let differences: String = codes.iter().map(|(c, n)| format!("{c} /{n} ")).collect();
        vec![
            format!(
                "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 500 700] \
                 /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << >> \
                 /Encoding << /Type /Encoding /Differences [{differences}] >> >>"
            )
            .into_bytes(),
        ]
    }

    fn type3_texts(codes: &[(u8, &str)]) -> Vec<String> {
        let shown: Vec<u8> = codes.iter().map(|(c, _)| *c).collect();
        read_codes(&type3(codes), None, &shown)
    }

    #[test]
    fn glyph_names_read_by_the_agl_specification() {
        let rows: &[(u8, &str, &str)] = &[
            (0x80, "A", "A"),
            (0x81, "Euro", "€"),
            (0x82, "Acircumflex", "Â"),
            (0x83, "afii10017", "\u{410}"),
            (0x84, "uni0041", "A"),
            (0x85, "uni00e9", "é"),
            (0x86, "u00E9", "é"),
            (0x87, "u0041", "A"),
            (0x88, "uni004", FFFD),
            (0x89, "uFFFFFF", FFFD),
            (0x8A, "foo", FFFD),
            (0x8B, "g65", FFFD),
            (0x8C, "G41", FFFD),
            (0x8D, "cid65", FFFD),
            (0x8E, "a1", FFFD),
            (0x8F, "A.sc", "A"),
            (0x90, "f_i", "fi"),
            (0x91, "f_f_i", "ffi"),
            (0x92, "a5", FFFD),
            (0x93, "space", " "),
            (0x94, "Omega", "\u{2126}"),
            (0x95, "Delta", "\u{2206}"),
            (0x96, "mu", "\u{b5}"),
            (0x97, "fi", "fi"),
            (0x98, "uni0000", FFFD),
            (0x41, "sfthyphen", "\u{ad}"),
            (0x42, "ff", "ff"),
            // Beyond MuPDF, toward the AGL specification.
            (0x99, "uni00410042", "AB"),
            (0x9A, "uniD800", FFFD),
            (0x9B, "u1F600", "\u{1F600}"),
            (0x9C, "T_h", "Th"),
            (0x9D, "uni0041_uni0042", "AB"),
            (0xA2, "a67", FFFD),
            (0x9E, "dalethatafpatah", "\u{5d3}\u{5b2}"),
            (0x9F, "uni0009", " "),
            (0xA0, "u0007", FFFD),
            (0xA1, "uni0041_foo", FFFD),
            (0xA3, "nbspace", "\u{a0}"),
            (0xA4, "delta", "\u{3b4}"),
            (0xA5, ".notdef", FFFD),
            (0xA6, "uni00410", FFFD),
            (0xA7, "u+0041", FFFD),
            (0xA8, "u10FFFF", "\u{10FFFF}"),
            (0xA9, "u110000", FFFD),
        ];
        let codes: Vec<(u8, &str)> = rows.iter().map(|(c, n, _)| (*c, *n)).collect();
        let texts = type3_texts(&codes);
        let wrong: Vec<String> = rows
            .iter()
            .zip(&texts)
            .filter(|((_, _, want), got)| got != want)
            .map(|((_, name, want), got)| format!("/{name}: {got:?} not {want:?}"))
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn glyph_name_api() {
        assert_eq!(glyph_name(b"fi", false), Some(vec!['\u{FB01}']));
        assert_eq!(glyph_name(b"a20", false), None);
        assert_eq!(glyph_name(b"a20", true), Some(vec!['\u{2714}']));
        assert_eq!(glyph_name(b"A", true), Some(vec!['A']));
        assert_eq!(glyph_name(b"", false), None);
        assert_eq!(glyph_name(b"_", false), None);
        assert_eq!(glyph_name(b"uni", false), None);
        assert_eq!(glyph_name(b"\xff\x00", false), None);
    }

    #[test]
    fn type3_names_that_are_codes() {
        assert_eq!(type3_texts(&[(65, "a65"), (200, "a200")]), ["A", "\u{c8}"]);
        assert_eq!(
            type3_texts(&[
                (162, "a67"),
                (65, "g65"),
                (66, "a65"),
                (67, "g67"),
                (66, "a066")
            ]),
            [FFFD, FFFD, FFFD, FFFD, FFFD]
        );
        // A control code's own name reads as nothing, so U+FFFD.
        assert_eq!(type3_texts(&[(7, "a7")]), [FFFD]);
    }

    #[test]
    fn type3_page_of_the_view_tests() {
        let names = [
            "uni0041", "a67", "A.sc", "f_i", "T_h", "foo", "a5", "u00E9", "alpha",
        ];
        let mut codes = vec![(32u8, "space"), (65, "A"), (66, "B")];
        codes.extend(names.iter().enumerate().map(|(i, n)| (161 + i as u8, *n)));
        let texts = read_codes(
            &type3(&codes),
            None,
            b"\xa1\xa2\xa3\xa4\xa5\xa6\xa7\xa8\xa9\x41\xa0",
        );
        assert_eq!(
            texts,
            ["A", FFFD, "A", "fi", "Th", FFFD, FFFD, "é", "α", "A", FFFD]
        );
    }

    // ------------------------------------------------------------ N2, N9 ToUnicode

    fn to_unicode_text(to_unicode: &str) -> String {
        let (entry, extra) = if to_unicode.starts_with('/') && !to_unicode.contains(' ') {
            (to_unicode.to_string(), vec![])
        } else {
            ("4 0 R".to_string(), vec![stream("", to_unicode.as_bytes())])
        };
        let mut objects = vec![
            format!(
                "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica \
                 /Encoding /WinAnsiEncoding /ToUnicode {entry} >>"
            )
            .into_bytes(),
        ];
        objects.extend(extra);
        read(&objects, None, b"ABCD")
    }

    fn cmap(body: &str) -> String {
        format!(
            "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T def \
             1 begincodespacerange <00> <FF> endcodespacerange\n{body}\n\
             endcmap CMapName currentdict /CMap defineresource pop end end"
        )
    }

    #[test]
    fn to_unicode_cmaps_map_codes_and_fall_back_for_the_rest() {
        let cases: Vec<(String, &str)> = vec![
            (cmap("1 beginbfchar <41> <0058> endbfchar"), "XBCD"),
            (cmap("1 beginbfchar <41> <00660069> endbfchar"), "fiBCD"),
            (
                cmap("1 beginbfchar <41> <D83DDE00> endbfchar"),
                "\u{1F600}BCD",
            ),
            (cmap("1 beginbfrange <41> <43> <0061> endbfrange"), "abcD"),
            (
                cmap("1 beginbfrange <41> <43> <00660069> endbfrange"),
                "fifjfkD",
            ),
            (
                cmap("1 beginbfrange <41> <43> [<0078> <00790079> <007A>] endbfrange"),
                "xyyzD",
            ),
            (cmap("1 beginbfchar <42> <0058> endbfchar"), "AXCD"),
            (
                cmap("2 beginbfchar <41> <0058> <41> <0059> endbfchar"),
                "YBCD",
            ),
            (cmap("1 begincidchar <41> 72 endcidchar"), "HBCD"),
            (
                "begincmap 1 beginbfchar <41> <0058> endbfchar endcmap".into(),
                "XBCD",
            ),
            (cmap("1 beginbfchar <41> <0000> endbfchar"), "ABCD"),
            (cmap("1 beginbfchar <41> <0007> endbfchar"), "ABCD"),
            (cmap("1 beginbfchar <41> <000A> endbfchar"), " BCD"),
            (cmap("1 beginbfchar <41> <0009> endbfchar"), " BCD"),
            (cmap("1 beginbfchar <41> <> endbfchar"), "ABCD"),
            (cmap("1 beginbfchar <41> <5A> endbfchar"), "ZBCD"),
            (cmap("1 beginbfchar <41> <D800> endbfchar"), "ABCD"),
            (
                cmap("1 beginbfrange <41> <43> <00FF> endbfrange"),
                "\u{ff}\u{100}\u{101}D",
            ),
            (
                "begincmap 1 begincodespacerange <0000> <FFFF> endcodespacerange \
                 1 beginbfchar <0041> <0058> endbfchar endcmap"
                    .into(),
                "XBCD",
            ),
            (
                cmap("> junk 1 beginbfchar <41> <0058> endbfchar ] stray"),
                "XBCD",
            ),
            ("this is not a cmap".into(), "ABCD"),
            // Tolerant entries (3.3) and controls in sequences (3.7).
            (cmap("2 beginbfchar <41> /H <42> <0058> endbfchar"), "AXCD"),
            (cmap("1 beginbfchar <41> <004100000042> endbfchar"), "ABBCD"),
            // A three-byte destination is padded: 0041 4200.
            (
                cmap("1 beginbfchar <41> <004142> endbfchar"),
                "A\u{4200}BCD",
            ),
            // A later range replaces part of an earlier one.
            (
                cmap(
                    "1 beginbfrange <41> <44> <0061> endbfrange 1 beginbfchar <42> <0058> endbfchar",
                ),
                "aXcd",
            ),
            // A malformed range and a range past its array are skipped.
            (
                cmap("2 beginbfrange <43> <41> <0061> <41> <42> [<0078>] endbfrange"),
                "xBCD",
            ),
            (cmap("1 beginbfchar <41> <00zz> endbfchar"), "ABCD"),
            (cmap("1 beginbfchar <41> <0058"), "ABCD"),
        ];
        let wrong: Vec<String> = cases
            .iter()
            .filter(|(to_unicode, want)| to_unicode_text(to_unicode) != *want)
            .map(|(to_unicode, want)| {
                format!(
                    "{to_unicode}: {:?} not {want:?}",
                    to_unicode_text(to_unicode)
                )
            })
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
        assert_eq!(to_unicode_text("/Identity-H"), "ABCD");
    }

    fn identity_font_text(to_unicode: &str) -> String {
        read(
            &[
                b"<< /Type /Font /Subtype /Type0 /BaseFont /Helvetica /Encoding /Identity-H \
                  /DescendantFonts [4 0 R] /ToUnicode 5 0 R >>"
                    .to_vec(),
                b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Helvetica \
                  /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
                  /FontDescriptor 6 0 R >>"
                    .to_vec(),
                stream("", to_unicode.as_bytes()),
                b"<< /Type /FontDescriptor /FontName /Helvetica /Flags 32 \
                  /FontBBox [0 -200 1000 800] /ItalicAngle 0 /Ascent 800 /Descent -200 \
                  /CapHeight 700 /StemV 80 >>"
                    .to_vec(),
            ],
            None,
            b"\x00\x01\x00\x02\x00\x03\x00\x03",
        )
    }

    #[test]
    fn to_unicode_cmaps_read_leniently() {
        let stray = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                     /CIDSystemInfo << /Registry (Adobe) /Ordering (UC>) /Supplement 0 >> def \
                     /CMapName /Adobe-Identity-UC> def 1 begincodespacerange <0000> <FFFF> \
                     endcodespacerange 3 beginbfchar <0001> <0048> <0002> <0065> <0003> <006C> \
                     endbfchar endcmap";
        assert_eq!(identity_font_text(stray), "Hell");
        let cids = "begincmap 1 begincodespacerange <0000> <FFFF> endcodespacerange \
                    1 begincidrange <0001> <0003> 72 endcidrange endcmap";
        assert_eq!(identity_font_text(cids), "HIJJ");
        // A malformed entry is skipped; CIDs 1, 2, 3 then read through the
        // Macintosh order: .null, nonmarkingreturn, space.
        let broken = "begincmap 1 beginbfchar <0001> /H endbfchar endcmap";
        assert_eq!(identity_font_text(broken), "\u{fffd}\u{fffd}  ");
    }

    #[test]
    fn to_unicode_usecmap_follows_predefined_cmaps() {
        // Adobe-Japan1-UCS2 maps CIDs 0x42 to 0x44 to a, b, c; the stream
        // maps 0x41 itself.
        let text = to_unicode_text(&cmap(
            "/Adobe-Japan1-UCS2 usecmap 1 beginbfchar <41> <0058> endbfchar",
        ));
        assert_eq!(text, "Xabc");
        let map = UnicodeMap::parse(
            b"/Adobe-Japan1-UCS2 usecmap 1 beginbfchar <41> <0058> endbfchar",
            &mut CMaps::new(),
        );
        assert_eq!(map.lookup(0x22), Some(vec!['A']));
        assert_eq!(map.lookup(0x41), Some(vec!['X']));
    }

    #[test]
    fn a_stray_begin_keyword_ends_its_block_and_starts_the_next() {
        // A bfchar block without its end: the bfrange after it still maps.
        let map = UnicodeMap::parse(
            b"1 beginbfchar <41> <0058> \
              1 beginbfrange <42> <43> <0061> endbfrange \
              1 beginbfchar <44> <0059> begincidchar <45> 72 endcidchar",
            &mut CMaps::new(),
        );
        assert_eq!(map.lookup(0x41), Some(vec!['X']));
        assert_eq!(map.lookup(0x42), Some(vec!['a']));
        assert_eq!(map.lookup(0x43), Some(vec!['b']));
        assert_eq!(map.lookup(0x44), Some(vec!['Y']));
        assert_eq!(map.lookup(0x45), Some(vec!['H']));
        // A begin keyword that opens no mapping block is skipped.
        let map = UnicodeMap::parse(
            b"1 beginbfchar <41> <0058> begincmap <42> <0059> endbfchar",
            &mut CMaps::new(),
        );
        assert_eq!(map.lookup(0x41), Some(vec!['X']));
        assert_eq!(map.lookup(0x42), None);
    }

    #[test]
    fn to_unicode_large_ranges_are_one_entry() {
        let map = UnicodeMap::parse(
            b"1 beginbfrange <00000000> <FFFFFFFF> <0041> endbfrange \
              1 beginbfchar <00000010> <0058> endbfchar",
            &mut CMaps::new(),
        );
        assert_eq!(map.entries.len(), 2);
        assert_eq!(map.intervals.len(), 3);
        assert_eq!(map.lookup(0x10), Some(vec!['X']));
        assert_eq!(map.lookup(0x11), Some(vec!['R']));
        // Past the last unit's range: no text.
        assert_eq!(map.lookup(0xFFFF_FFFF), None);
        assert_eq!(map.lookup(1), Some(vec!['B']));
    }

    // ------------------------------------------------------------ N3 Symbol, ZapfDingbats

    fn standard_font_texts(base: &str) -> Vec<String> {
        let shown: Vec<u8> = (0x20..=0xFE).collect();
        read_codes(
            &[format!("<< /Type /Font /Subtype /Type1 /BaseFont /{base} >>").into_bytes()],
            None,
            &shown,
        )
    }

    fn check_built_in(base: &str, table: &[(u8, &[u32])]) {
        let texts = standard_font_texts(base);
        let mut wrong = Vec::new();
        for code in 0x20..=0xFEu8 {
            let got = &texts[usize::from(code - 0x20)];
            match table.iter().find(|(c, _)| *c == code) {
                Some((_, values)) => {
                    if !values.iter().any(|&v| *got == u(&[v])) {
                        wrong.push(format!("{code:#04X}: {got:?} not {values:X?}"));
                    }
                }
                None => {
                    if got != FFFD {
                        wrong.push(format!("{code:#04X}: {got:?} not U+FFFD"));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{base}: {}", wrong.join(", "));
    }

    #[test]
    fn symbol_and_zapf_dingbats_read_through_their_built_in_encodings() {
        check_built_in("Symbol", &SYMBOL);
        check_built_in("ABCDEF+SymbolMT,Bold", &SYMBOL);
        check_built_in("ZapfDingbats", &DINGBATS);
        check_built_in("Dingbats", &DINGBATS);
        assert_eq!(standard_font_texts("ZapfDingbats")[0x34 - 0x20], "\u{2714}");
        // G2: Symbol 0x60 and 0xA0.
        let symbol = standard_font_texts("Symbol");
        assert_eq!(symbol[0x60 - 0x20], "\u{F8E5}");
        assert_eq!(symbol[0xA0 - 0x20], "\u{20AC}");
        assert_eq!(symbol[0x6D - 0x20], "\u{3BC}");
        assert_eq!(symbol[0x57 - 0x20], "\u{3A9}");
    }

    #[test]
    fn non_embedded_simple_fonts_read_their_encodings() {
        // G2: codes 0x41 0x60 0x7F 0x80 0xA0 0xA4 0xAD 0xC6.
        let shown = b"\x41\x60\x7f\x80\xa0\xa4\xad\xc6";
        let font = |encoding: &str| {
            vec![
                format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica {encoding} >>")
                    .into_bytes(),
            ]
        };
        // 0xC6 is breve in StandardEncoding (Annex D.2, Adobe's
        // stdenc.txt); G2 records U+02DC for this cell.
        let standard = [
            "A", "\u{2018}", FFFD, FFFD, FFFD, "\u{2044}", "\u{203A}", "\u{2D8}",
        ];
        assert_eq!(read_codes(&font(""), None, shown), standard);
        assert_eq!(
            read_codes(&font("/Encoding /FooEncoding"), None, shown),
            standard
        );
        assert_eq!(
            read_codes(&font("/Encoding /WinAnsiEncoding"), None, shown),
            ["A", "`", "\u{2022}", "\u{20AC}", " ", "¤", "-", "Æ"]
        );
        assert_eq!(
            read_codes(&font("/Encoding /MacRomanEncoding"), None, shown),
            ["A", "`", FFFD, "Ä", "\u{2020}", "§", "\u{203A}", "\u{2D8}"]
        );
        let mut overlay = standard;
        overlay[0] = "\u{2126}";
        assert_eq!(
            read_codes(
                &font("/Encoding << /Differences [65 /Omega] >>"),
                None,
                shown
            ),
            overlay
        );
        assert_eq!(
            read_codes(
                &font("/Encoding << /BaseEncoding /Foo /Differences [65 /Omega] >>"),
                None,
                shown
            ),
            overlay
        );
        // An Arial TrueType font without a program, symbolic or not.
        for flags in [32, 4] {
            let arial = format!(
                "<< /Type /Font /Subtype /TrueType /BaseFont /Arial /FontDescriptor \
                 << /Type /FontDescriptor /FontName /Arial /Flags {flags} >> >>"
            );
            assert_eq!(read_codes(&[arial.into_bytes()], None, shown), standard);
        }
        // An embedded Type1 program: the interpreter reads codes without a
        // name.
        let embedded = b"<< /Type /Font /Subtype /Type1 /BaseFont /X \
            /Encoding << /Differences [65 /B] >> \
            /FontDescriptor << /FontName /X /Flags 32 /FontFile 4 0 R >> >>"
            .to_vec();
        assert_eq!(
            read_codes(&[embedded, stream("", b"")], Some(b"%!PS"), b"AB"),
            ["B", "<none>"]
        );
    }

    #[test]
    fn encoding_names_follow_the_font() {
        let names = |dict: &str| {
            let pdf = Pdf::new(pdf(&[dict.as_bytes().to_vec()])).unwrap();
            let font = pdf
                .xref()
                .get::<Dict<'_>>(ObjectIdentifier::new(3, 0))
                .unwrap();
            encoding_names(&font)
        };
        let helvetica = names("<< /Subtype /Type1 /BaseFont /Helvetica >>");
        assert_eq!(helvetica[0x60].as_deref(), Some(b"quoteleft".as_slice()));
        let type3 =
            names("<< /Subtype /Type3 /Encoding << /Differences [65 /a /b 300 /c 255 /d /e] >> >>");
        assert_eq!(type3[65].as_deref(), Some(b"a".as_slice()));
        assert_eq!(type3[66].as_deref(), Some(b"b".as_slice()));
        assert_eq!(type3[67], None);
        assert_eq!(type3[255].as_deref(), Some(b"d".as_slice()));
        assert_eq!(type3.iter().flatten().count(), 3);
        let symbolic = names("<< /Subtype /Type1 /BaseFont /Foo /FontDescriptor << /Flags 4 >> >>");
        assert!(symbolic.iter().all(Option::is_none));
        let composite = names("<< /Subtype /Type0 /Encoding /Identity-H >>");
        assert_eq!(composite.len(), 256);
        assert!(composite.iter().all(Option::is_none));
    }

    // ------------------------------------------------------------ N4, N5 TrueType programs

    fn truetype(mut tables: Vec<(&[u8; 4], Vec<u8>)>) -> Vec<u8> {
        tables.sort_by_key(|(tag, _)| **tag);
        let mut font = Vec::new();
        font.extend(0x0001_0000u32.to_be_bytes());
        for v in [tables.len() as u16, 0, 0, 0] {
            font.extend(v.to_be_bytes());
        }
        let mut offset = 12 + 16 * tables.len();
        for (tag, data) in &tables {
            font.extend(*tag);
            for v in [0, offset as u32, data.len() as u32] {
                font.extend(v.to_be_bytes());
            }
            offset += data.len().next_multiple_of(4);
        }
        for (_, data) in &tables {
            font.extend(data);
            font.resize(font.len().next_multiple_of(4), 0);
        }
        font
    }

    fn be16s(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn post_v2(names: &[&str]) -> Vec<u8> {
        let mut post = Vec::new();
        post.extend(0x0002_0000u32.to_be_bytes());
        post.extend([0u8; 28]);
        post.extend((names.len() as u16 + 1).to_be_bytes());
        post.extend(0u16.to_be_bytes());
        for i in 0..names.len() {
            post.extend((258 + i as u16).to_be_bytes());
        }
        for name in names {
            post.push(name.len() as u8);
            post.extend(name.as_bytes());
        }
        post
    }

    fn cmap_table(platform: u16, encoding: u16, subtable: Vec<u8>) -> Vec<u8> {
        let mut cmap = be16s(&[0, 1, platform, encoding]);
        cmap.extend(12u32.to_be_bytes());
        cmap.extend(subtable);
        cmap
    }

    fn format4(segments: &[(u16, u16, i16)]) -> Vec<u8> {
        let mut all = segments.to_vec();
        all.push((0xFFFF, 0xFFFF, 1));
        let n = all.len() as u16;
        let mut data = be16s(&[0, 0, 0, 2 * n, 0, 0, 0]);
        data.extend(all.iter().flat_map(|s| s.1.to_be_bytes()));
        data.extend([0, 0]);
        data.extend(all.iter().flat_map(|s| s.0.to_be_bytes()));
        data.extend(all.iter().flat_map(|s| s.2.to_be_bytes()));
        data.extend(all.iter().flat_map(|_| 0u16.to_be_bytes()));
        let length = data.len() as u16;
        data[2..4].copy_from_slice(&length.to_be_bytes());
        data[0..2].copy_from_slice(&4u16.to_be_bytes());
        data
    }

    fn format12(groups: &[(u32, u32, u32)], claimed: u32) -> Vec<u8> {
        let mut data = be16s(&[12, 0]);
        data.extend((16 + 12 * groups.len() as u32).to_be_bytes());
        data.extend(0u32.to_be_bytes());
        data.extend(claimed.to_be_bytes());
        for (start, end, glyph) in groups {
            for v in [start, end, glyph] {
                data.extend(v.to_be_bytes());
            }
        }
        data
    }

    fn embedded_truetype(program: &[u8], flags: u32, shown: &[u8]) -> String {
        read(
            &[
                format!(
                    "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Test \
                     /FontDescriptor << /Type /FontDescriptor /FontName /ABCDEF+Test \
                     /Flags {flags} /FontFile2 4 0 R >> >>"
                )
                .into_bytes(),
                stream("", program),
            ],
            Some(program),
            shown,
        )
    }

    fn glyf_tables() -> Vec<(&'static [u8; 4], Vec<u8>)> {
        vec![(b"glyf", vec![]), (b"head", vec![0; 54])]
    }

    #[test]
    fn format_4_segments_decide_in_table_order() {
        let mut tables = glyf_tables();
        tables.push((
            b"cmap",
            cmap_table(
                3,
                1,
                format4(&[(0x44, 0x46, 4 - 0x44), (0x41, 0x45, 1 - 0x41)]),
            ),
        ));
        tables.push((
            b"post",
            post_v2(&["alpha", "beta", "gamma", "Delta", "Epsilon", "Phi"]),
        ));
        assert_eq!(
            embedded_truetype(&truetype(tables), 32, b"ABCDEF"),
            "\u{fffd}\u{fffd}\u{fffd}\u{2206}\u{395}\u{3a6}"
        );
    }

    #[test]
    fn format_4_names_glyphs_through_post_format_2() {
        let mut tables = glyf_tables();
        tables.push((
            b"cmap",
            cmap_table(3, 1, format4(&[(0x41, 0x43, 1 - 0x41)])),
        ));
        tables.push((b"post", post_v2(&["alpha", "beta", "gamma"])));
        assert_eq!(
            embedded_truetype(&truetype(tables), 32, b"ABC"),
            "\u{3b1}\u{3b2}\u{3b3}"
        );
    }

    #[test]
    fn format_12_groups_read_what_the_table_holds() {
        let mut tables = glyf_tables();
        tables.push((
            b"cmap",
            cmap_table(3, 10, format12(&[(0x41, 0x42, 1), (0x43, 0x43, 3)], 5)),
        ));
        tables.push((b"post", post_v2(&["alpha", "beta", "gamma"])));
        assert_eq!(
            embedded_truetype(&truetype(tables), 32, b"ABC"),
            "\u{3b1}\u{3b2}\u{3b3}"
        );
    }

    #[test]
    fn a_symbolic_font_reads_ascii_codes_through_its_3_0_subtable() {
        let mut tables = glyf_tables();
        tables.push((
            b"cmap",
            cmap_table(3, 0, format4(&[(0xF041, 0xF043, (1i32 - 0xF041) as i16)])),
        ));
        tables.push((b"post", post_v2(&["Alpha", "Beta", "Gamma"])));
        let program = truetype(tables);
        assert_eq!(
            embedded_truetype(&program, 4, b"ABCD"),
            "\u{391}\u{392}\u{393}\u{fffd}"
        );
    }

    #[test]
    fn post_formats_1_and_3_and_unnamed_glyphs() {
        // Format 1: the standard order (glyph 36 is A).
        let mut post = 0x0001_0000u32.to_be_bytes().to_vec();
        post.extend([0u8; 28]);
        let mut tables = glyf_tables();
        tables.push((
            b"cmap",
            cmap_table(3, 1, format4(&[(0x41, 0x42, 36 - 0x41)])),
        ));
        tables.push((b"post", post));
        assert_eq!(embedded_truetype(&truetype(tables), 32, b"AB"), "AB");
        // Format 3: no names. A symbolic font's printable ASCII codes read
        // as themselves; a non-symbolic font's do not.
        let mut post = 0x0003_0000u32.to_be_bytes().to_vec();
        post.extend([0u8; 28]);
        let mut tables = glyf_tables();
        tables.push((b"cmap", cmap_table(3, 0, format4(&[(0x20, 0x7F, 1)]))));
        tables.push((b"post", post));
        let program = truetype(tables);
        assert_eq!(embedded_truetype(&program, 4, b"Hi!\x7f"), "Hi!\u{fffd}");
        assert_eq!(embedded_truetype(&program, 32, b"Hi"), "\u{fffd}\u{fffd}");
        // A CFF-based program is left to the interpreter.
        let mut otto = truetype(vec![(b"CFF ", vec![0; 8])]);
        otto[0..4].copy_from_slice(b"OTTO");
        assert_eq!(embedded_truetype(&otto, 32, b"A"), "<none>");
    }

    #[test]
    fn truetype_cmaps_with_many_groups_are_read_by_bisection() {
        const GROUPS: u32 = 500_000;
        let groups: Vec<(u32, u32, u32)> =
            (0..GROUPS).map(|i| (0x10000 + i, 0x10000 + i, 1)).collect();
        let program = truetype(vec![
            (b"cmap", cmap_table(3, 1, format12(&groups, GROUPS))),
            (b"glyf", vec![0; 4]),
        ]);
        let objects = vec![
            b"<< /Type /Font /Subtype /TrueType /BaseFont /T /FontDescriptor \
              << /Type /FontDescriptor /FontName /T /Flags 32 /FontFile2 4 0 R >> >>"
                .to_vec(),
            stream("", b""),
        ];
        let started = Instant::now();
        for _ in 0..8 {
            let font = font_of(&objects, Some(&program));
            assert_eq!(
                font.text(Code {
                    code: 65,
                    cid: 65,
                    bytes: 1
                }),
                Some(vec!['\u{fffd}'])
            );
        }
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn hostile_font_programs_do_not_panic() {
        let mut tables = glyf_tables();
        tables.push((
            b"cmap",
            cmap_table(3, 1, format4(&[(0x41, 0x43, 1 - 0x41)])),
        ));
        tables.push((b"post", post_v2(&["alpha", "beta", "gamma"])));
        let program = truetype(tables);
        let mut seed = 7u64;
        for cut in 0..program.len() {
            let _ = embedded_truetype(&program[..cut], 4, b"ABC");
            let mut noisy = program.clone();
            for _ in 0..4 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let at = (seed >> 33) as usize % noisy.len();
                noisy[at] = (seed >> 13) as u8;
            }
            let _ = TrueTypeReader::new(&noisy, true).map(|r| r.reading(0x41));
            if let Some(sfnt) = Sfnt::parse(&noisy) {
                if let Some(cmap) = sfnt.table(b"cmap") {
                    for subtable in cmap_subtables(cmap) {
                        subtable.each_mapping(&mut |_, _| {});
                        let _ = subtable.glyph(0x10FFFF);
                    }
                }
            }
        }
    }

    #[test]
    fn hostile_cmaps_do_not_panic() {
        let base = cmap(
            "2 beginbfrange <41> <43> [<0078> <00790079>] <00> <FF> <D83DDFFF> endbfrange \
             1 begincidrange <0000> <FFFF> 4294967295 endcidrange (unclosed",
        )
        .into_bytes();
        let mut seed = 3u64;
        for cut in 0..base.len() {
            let map = UnicodeMap::parse(&base[..cut], &mut CMaps::new());
            for code in [0, 0x41, 0x42, 0x43, 0xFF, 0xFFFF, u32::MAX] {
                let _ = map.lookup(code);
            }
            let mut noisy = base.clone();
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let at = (seed >> 33) as usize % noisy.len();
            noisy[at] = (seed >> 13) as u8;
            let map = UnicodeMap::parse(&noisy, &mut CMaps::new());
            let _ = map.lookup(0x41);
        }
    }

    // ------------------------------------------------------------ N6, N7 composite fonts

    fn composite_objects(
        encoding: &str,
        subtype: &str,
        base: &str,
        ordering: &str,
    ) -> Vec<Vec<u8>> {
        vec![
            format!(
                "<< /Type /Font /Subtype /Type0 /BaseFont /{base} /Encoding /{encoding} \
                 /DescendantFonts [4 0 R] >>"
            )
            .into_bytes(),
            format!(
                "<< /Type /Font /Subtype /{subtype} /BaseFont /{base} \
                 /CIDSystemInfo << /Registry (Adobe) /Ordering ({ordering}) /Supplement 2 >> \
                 /FontDescriptor 5 0 R >>"
            )
            .into_bytes(),
            format!(
                "<< /Type /FontDescriptor /FontName /{base} /Flags 4 \
                 /FontBBox [0 -120 1000 880] /ItalicAngle 0 /Ascent 880 /Descent -120 \
                 /CapHeight 700 /StemV 80 >>"
            )
            .into_bytes(),
        ]
    }

    fn composite_text(
        encoding: &str,
        subtype: &str,
        base: &str,
        ordering: &str,
        shown: &[u8],
    ) -> String {
        read(
            &composite_objects(encoding, subtype, base, ordering),
            None,
            shown,
        )
    }

    #[test]
    fn cid_fonts_without_to_unicode_read_through_their_collection() {
        let rows = [
            ("Japan1", [0x41, 0x3B6, 0x308D]),
            ("GB1", [0x41, 0x62B1, 0x5DFC]),
            ("CNS1", [0x41, 0x8840, 0x9A04]),
            ("Korea1", [0x41, 0x423, 0xBB23]),
        ];
        for (ordering, want) in rows {
            assert_eq!(
                composite_text(
                    "Identity-H",
                    "CIDFontType0",
                    "Song",
                    ordering,
                    b"\x00\x22\x04\x10\x30\x42"
                ),
                u(&want),
                "{ordering}"
            );
        }
        assert_eq!(
            composite_text(
                "90ms-RKSJ-H",
                "CIDFontType0",
                "Song",
                "Japan1",
                b"\x82\xa0\x41\x42\x42"
            ),
            "\u{3042}ABB"
        );
        assert_eq!(
            composite_text(
                "Identity-H",
                "CIDFontType2",
                "Arial",
                "Identity",
                b"\x00\x24\x00\x44"
            ),
            "Aa"
        );
        // CID 258 and past: no standard Macintosh name.
        assert_eq!(
            composite_text(
                "Identity-H",
                "CIDFontType2",
                "Arial",
                "Identity",
                b"\x01\x02"
            ),
            FFFD
        );
    }

    #[test]
    fn cff_cid_fonts_with_identity_ordering_read_as_unknown() {
        assert_eq!(
            composite_text(
                "Identity-H",
                "CIDFontType0",
                "Song",
                "Identity",
                b"\x00\x22\x04\x10\x30\x42"
            ),
            "\u{fffd}\u{fffd}\u{fffd}"
        );
    }

    #[test]
    fn a_utf16_cmap_reads_a_surrogate_pair_as_one_character() {
        let objects = composite_objects("UniGB-UTF16-H", "CIDFontType0", "STSong-Light", "GB1");
        let font = font_of(&objects, None);
        let mut codes = Vec::new();
        font.codes(b"\x65\xe5\xd8\x40\xdc\x00\x30\x02", &mut codes);
        assert_eq!(
            codes.iter().map(|c| (c.code, c.bytes)).collect::<Vec<_>>(),
            [(0x65E5, 2), (0xD840_DC00, 4), (0x3002, 2)]
        );
        assert_eq!(
            read(&objects, None, b"\x65\xe5\xd8\x40\xdc\x00\x30\x02"),
            "\u{65e5}\u{20000}\u{3002}"
        );
        // An unpaired surrogate and an odd last byte.
        assert_eq!(
            read_codes(&objects, None, b"\xd8\x40\x00\x41\x30"),
            [FFFD, "A", FFFD]
        );
        let ucs2 = composite_objects("UniJIS-UCS2-H", "CIDFontType0", "Song", "Japan1");
        assert_eq!(read(&ucs2, None, b"\x30\x42\xe0\x00"), "\u{3042}\u{e000}");
    }

    #[test]
    fn identity_to_unicode_reads_codes_as_utf_16() {
        let objects = vec![
            b"<< /Type /Font /Subtype /Type0 /BaseFont /Song /Encoding /Identity-H \
              /DescendantFonts [4 0 R] /ToUnicode /Identity-H >>"
                .to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Song \
              /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
              /FontDescriptor << /Type /FontDescriptor /FontName /Song /Flags 4 >> >>"
                .to_vec(),
        ];
        assert_eq!(read(&objects, None, b"\x00\x41\x00\x42"), "AB");
        // A surrogate code reads through the next source (Macintosh order:
        // 0xD800 is past it).
        assert_eq!(read(&objects, None, b"\xd8\x00"), FFFD);
    }

    #[test]
    fn embedded_cid_truetype_programs_read_their_inverted_cmap() {
        let mut tables = glyf_tables();
        // Glyphs 1..3 for A..C; glyph 3 also for U+00C7: two characters,
        // so no reading.
        tables.push((
            b"cmap",
            cmap_table(
                3,
                1,
                format4(&[(0x41, 0x43, 1 - 0x41), (0xC7, 0xC7, 3 - 0xC7)]),
            ),
        ));
        let program = truetype(tables);
        let objects = |map: &str| {
            vec![
                b"<< /Type /Font /Subtype /Type0 /BaseFont /T /Encoding /Identity-H \
                  /DescendantFonts [4 0 R] >>"
                    .to_vec(),
                format!(
                    "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /T \
                     /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
                     {map} /FontDescriptor << /FontName /T /Flags 4 /FontFile2 5 0 R >> >>"
                )
                .into_bytes(),
                stream("", &program),
                stream("", &be16s(&[0, 2, 1])),
            ]
        };
        assert_eq!(
            read_codes(
                &objects(""),
                Some(&program),
                b"\x00\x01\x00\x02\x00\x03\x00\x09"
            ),
            ["A", "B", "<none>", "<none>"]
        );
        assert_eq!(
            read(
                &objects("/CIDToGIDMap 6 0 R"),
                Some(&program),
                b"\x00\x01\x00\x02"
            ),
            "BA"
        );
        // Without a usable program: the interpreter reads it.
        assert_eq!(read(&objects(""), None, b"\x00\x01"), "<none>");
    }

    #[test]
    fn inverting_a_format_4_table_of_many_segments_is_linear() {
        // 32,766 one-code segments (code i to glyph i + 1) and the closing
        // one: a lookup per code scanning the segments would cost about 10^9
        // reads for each font.
        let segments: Vec<(u16, u16, i16)> = (0..32_766u16).map(|i| (i, i, 1)).collect();
        let mut tables = glyf_tables();
        tables.push((b"cmap", cmap_table(3, 1, format4(&segments))));
        let program = truetype(tables);
        assert!(program.len() > 256_000);
        let objects = vec![
            b"<< /Type /Font /Subtype /Type0 /BaseFont /T /Encoding /Identity-H \
              /DescendantFonts [4 0 R] >>"
                .to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /T \
              /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
              /FontDescriptor << /FontName /T /Flags 4 /FontFile2 5 0 R >> >>"
                .to_vec(),
            stream("", b""),
        ];
        let started = Instant::now();
        for _ in 0..4 {
            assert_eq!(
                read_codes(&objects, Some(&program), b"\x00\x41\x00\x42\x80\x00"),
                ["@", "A", "<none>"]
            );
        }
        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "{:?}",
            started.elapsed()
        );
        // The same inversion through the subtable, against a lookup per code
        // on a small table with out-of-order, overlapping segments.
        let table = cmap_table(
            3,
            1,
            format4(&[
                (0x44, 0x46, 4 - 0x44),
                (0x41, 0x45, 1 - 0x41),
                (0x30, 0x50, 9),
            ]),
        );
        let subtables = cmap_subtables(&table);
        let mut walked = Vec::new();
        subtables[0].each_mapping(&mut |c, g| walked.push((c, g)));
        let looked: Vec<(u32, u16)> = (0..=0xFFFFu32)
            .map(|c| (c, subtables[0].glyph(c)))
            .filter(|p| p.1 != 0)
            .collect();
        assert_eq!(walked, looked);
    }

    #[test]
    fn codes_split_as_the_interpreter_splits_them() {
        let font = font_of(
            &composite_objects("Identity-H", "CIDFontType2", "T", "Identity"),
            None,
        );
        let mut codes = Vec::new();
        font.codes(b"\x01\x02\x03", &mut codes);
        assert_eq!(
            codes,
            [
                Code {
                    code: 0x102,
                    cid: 0x102,
                    bytes: 2
                },
                Code {
                    code: 0,
                    cid: 0,
                    bytes: 1
                },
            ]
        );
        let simple = font_of(&type3(&[(65, "A")]), None);
        let mut codes = Vec::new();
        simple.codes(b"AB", &mut codes);
        assert_eq!(
            codes[1],
            Code {
                code: 66,
                cid: 66,
                bytes: 1
            }
        );
        assert!(!simple.vertical());
    }

    #[test]
    fn vertical_writing_follows_the_cmap() {
        let vertical = |encoding: &str| {
            font_of(
                &composite_objects(encoding, "CIDFontType0", "Song", "Japan1"),
                None,
            )
            .vertical()
        };
        assert!(vertical("Identity-V"));
        assert!(!vertical("Identity-H"));
        assert!(vertical("90ms-RKSJ-V"));
        assert!(!vertical("90ms-RKSJ-H"));
        let embedded = |dict: &str, program: &str| {
            let objects = vec![
                b"<< /Type /Font /Subtype /Type0 /BaseFont /Song /Encoding 4 0 R \
                  /DescendantFonts [5 0 R] >>"
                    .to_vec(),
                stream(dict, program.as_bytes()),
                b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Song >>".to_vec(),
            ];
            font_of(&objects, None).vertical()
        };
        let program = |mode: u8| {
            format!(
                "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                 /CMapName /Test def /WMode {mode} def 1 begincodespacerange <0000> <FFFF> \
                 endcodespacerange 1 begincidrange <0000> <FFFF> 0 endcidrange endcmap \
                 CMapName currentdict /CMap defineresource pop end end"
            )
        };
        assert!(embedded("/Type /CMap", &program(1)));
        assert!(!embedded("/Type /CMap", &program(0)));
        assert!(embedded("/Type /CMap /WMode 1", &program(0)));
        assert!(!embedded("/Type /CMap /WMode 0", &program(1)));
    }

    // ------------------------------------------------------------ N8 normalisation

    #[test]
    fn presentation_forms_read_as_their_decompositions() {
        let normalised = |c: char| {
            let mut out = Vec::new();
            normalise_extracted(c, &mut out);
            out.into_iter().collect::<String>()
        };
        assert_eq!(normalised('\u{FB01}'), "fi");
        assert_eq!(normalised('\u{FB03}'), "ffi");
        assert_eq!(normalised('\u{FB2C}'), "\u{5e9}\u{5bc}\u{5c1}");
        assert_eq!(normalised('\u{FE8D}'), "\u{627}");
        assert_eq!(normalised('\u{FE10}'), ",");
        assert_eq!(normalised('\u{FEFF}'), "");
        assert_eq!(normalised('\u{FF21}'), "\u{ff21}");
        let changed = (0xFB00..=0xFEFFu32)
            .filter_map(char::from_u32)
            .filter(|&c| c != '\u{FEFF}' && normalised(c) != c.to_string())
            .count();
        assert_eq!(changed, 854);
        for (c, chars) in data::PRESENTATION_FORMS.iter() {
            assert!(
                !chars.is_empty() && chars.iter().all(|d| !('\u{FB00}'..='\u{FEFF}').contains(d)),
                "{c:?}"
            );
        }
        // Through a glyph name, as the acceptance test reads them.
        assert_eq!(
            type3_texts(&[(1, "uniFB2C"), (2, "uniFEFF")]),
            ["\u{5e9}\u{5bc}\u{5c1}", ""]
        );
    }

    // ------------------------------------------------------------ N11 data integrity

    #[test]
    fn generated_tables_are_whole_and_sorted() {
        assert_eq!(data::AGL_ENTRIES, 4281);
        assert_eq!(data::AGL.len(), data::AGL_ENTRIES);
        assert!(data::AGL.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(data::ZAPF_DINGBATS_LIST.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(data::SYMBOL_BUILTIN.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(data::DINGBATS_BUILTIN.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(data::PRESENTATION_FORMS.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(data::MIRRORS.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(data::MAC_GLYPH_ORDER.len(), 258);
        for (name, want) in [
            (b"Omega".as_slice(), '\u{2126}'),
            (b"Delta", '\u{2206}'),
            (b"mu", '\u{B5}'),
            (b"hyphen", '-'),
            (b"Euro", '\u{20AC}'),
            (b"fraction", '\u{2044}'),
        ] {
            assert_eq!(glyph_name(name, false), Some(vec![want]));
        }
    }

    #[test]
    fn bidi_classes_and_mirrors() {
        assert_eq!(bidi_class('\u{5D0}'), BidiClass::R);
        assert_eq!(bidi_class('\u{627}'), BidiClass::AL);
        assert_eq!(bidi_class('\u{661}'), BidiClass::AN);
        assert_eq!(bidi_class('1'), BidiClass::EN);
        assert_eq!(bidi_class('\u{10C80}'), BidiClass::R);
        assert_eq!(bidi_class('a'), BidiClass::L);
        assert_eq!(bidi_mirror('('), Some(')'));
        assert_eq!(bidi_mirror('»'), Some('«'));
        assert_eq!(bidi_mirror('a'), None);
    }

    /// The `unicode-bidi` crate's classes against the pinned
    /// DerivedBidiClass.txt (Unicode 16.0), defaults included.
    #[test]
    fn bidi_classes_match_the_derived_bidi_class_file() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/generate/data/DerivedBidiClass.txt"
        );
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let mut wrong = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((range, class)) = line.split_once(';') else {
                continue;
            };
            let (first, last) = match range.trim().split_once("..") {
                Some((a, b)) => (a, b),
                None => (range.trim(), range.trim()),
            };
            let first = u32::from_str_radix(first, 16).unwrap();
            let last = u32::from_str_radix(last, 16).unwrap();
            for c in (first..=last).filter_map(char::from_u32) {
                let got = format!("{:?}", bidi_class(c));
                if got != class.trim() {
                    wrong.push(format!("{:04X} {got} not {}", u32::from(c), class.trim()));
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{} differ: {:?}",
            wrong.len(),
            &wrong[..wrong.len().min(40)]
        );
    }
}
