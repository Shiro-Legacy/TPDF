// SPDX-License-Identifier: MIT
//! The font programs the source PDF already embeds, for the layout and the
//! emitter (design/app-xfa.md, Fonts). Owner decision: only fonts the
//! source embeds are used, never fonts of the machine and never fonts the
//! engine would ship; anything else draws with the base 14 substitute
//! (`text::face_for`) under its width scale.
//!
//! Discovery (`EmbeddedFonts::scan`): the font dictionaries of /AcroForm
//! /DR, of every page's /Resources and of the form XObjects those name
//! (four levels deep), each object once, at most `MAX_FONTS`. Used:
//!
//! - simple fonts (/Type1, /MMType1, /TrueType) whose descriptor has a
//!   /FontFile2 or a /FontFile3 of /Subtype /OpenType or /Type1C. A
//!   character is covered when the font's encoding (a named base,
//!   /Differences over it) gives it a code and the program has a glyph
//!   for it: through its Unicode cmap, else its (3,0) cmap at 0xF000 plus
//!   the code (or the code), else its (1,0) cmap at the code. A bare CFF
//!   program (Type1C) has no cmap: a code is covered when /Widths gives it
//!   a positive width.
//! - Type0 fonts with /Encoding /Identity-H whose descendant is a
//!   /CIDFontType2 with a TrueType or OpenType program carrying a Unicode
//!   cmap: a character is covered when the cmap maps it; its CID is its
//!   glyph (or, with a /CIDToGIDMap stream, the first CID mapped to it).
//!
//! Advances come from /Widths (simple) or /W and /DW (Type0), else from
//! the program's hmtx; ascent and descent from the descriptor, else hhea.
//! Programs past `MAX_PROGRAM_BYTES` (or past `MAX_TOTAL_PROGRAM_BYTES` in
//! all), and programs that do not read, are passed over quietly. Type1
//! programs (/FontFile) and CFF-keyed CID fonts are not used.
//!
//! Matching (`EmbeddedFonts::find`): the XFA family and the font's family
//! (from /BaseFont: subset tag dropped, `-Bold`, `,Italic`, `MT`, `PSMT`
//! split off; the descriptor's /Flags, /FontWeight and /ItalicAngle add
//! bold and italic) compare lowercased with everything but letters and
//! digits dropped, and the weight and posture must be the same. A run uses
//! the font only when it covers every character of the run.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::app::forms::inherited;
use crate::app::info;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::simple_encoding::{EncodingLimits, NamedEncoding, SimpleEncoding, registry_unicode};
use crate::stream::StreamLimits;
use crate::view::fonts::scan::SfntInfo;

use super::types::Font;

/// Fonts recorded at most.
pub const MAX_FONTS: usize = 64;
/// A font program's decoded bytes, at most.
pub const MAX_PROGRAM_BYTES: usize = 16 << 20;
/// All programs' decoded bytes together, at most.
pub const MAX_TOTAL_PROGRAM_BYTES: usize = 64 << 20;
/// Font dictionaries looked at, at most (recorded or not).
const MAX_FONT_ENTRIES: usize = 4096;
/// Pages whose resources are scanned, at most.
const MAX_PAGES: usize = 4096;
/// Form XObjects followed, at most, and how deep.
const MAX_XOBJECTS: usize = 1024;
const MAX_XOBJECT_DEPTH: usize = 4;
/// /W entries read, at most.
const MAX_CID_WIDTHS: usize = 65_536;
/// A /CIDToGIDMap stream's bytes, at most (every 16-bit CID).
const MAX_CID_MAP_BYTES: usize = 2 << 16;

/// Ascent and descent (1/1000 em) of a font that states neither, as
/// Helvetica's.
const DEFAULT_ASCENT: f64 = 718.0;
const DEFAULT_DESCENT: f64 = 207.0;

/// The embedded fonts of a document, in discovery order (an index is a
/// font's identity for `text::Piece::font` and its resource name).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EmbeddedFonts {
    fonts: Vec<EmbeddedFont>,
}

/// One embedded font.
#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddedFont {
    /// The font as the source names it (a reference, or a direct
    /// dictionary): the emitter copies it.
    pub object: Object,
    /// /BaseFont as written (a Type0 font's: its descendant's).
    pub base_font: String,
    /// `family_key` of the family /BaseFont names.
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    /// Above and below the baseline (positive), 1/1000 em.
    pub ascent: f64,
    pub descent: f64,
    pub encoding: Encoding,
}

/// How characters become codes.
#[derive(Clone, Debug, PartialEq)]
pub enum Encoding {
    /// A simple font: each covered character's one-byte code and advance
    /// (1/1000 em). `win_ansi`: /Encoding is the name /WinAnsiEncoding (a
    /// widget's /DA may name it: `forms::appearance` draws such fonts).
    Simple {
        codes: BTreeMap<char, (u8, f32)>,
        win_ansi: bool,
    },
    /// A Type0 font with /Identity-H: two-byte CIDs.
    Identity {
        /// The program's Unicode cmap: BMP character to glyph.
        glyphs: Vec<u16>,
        /// Glyph to CID, from a /CIDToGIDMap stream; None: the identity.
        cids: Option<HashMap<u16, u16>>,
        /// /W: CID to advance (1/1000 em); `default` (/DW) otherwise.
        widths: HashMap<u16, f32>,
        default: f32,
    },
}

/// A character's code in its font and its advance in 1/1000 em.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Code {
    pub code: u16,
    pub advance: f64,
}

/// What a character may be shown as when the font lacks it itself: the
/// space and hyphen variants WinAnsi folds the same way (`text::win_ansi_code`).
fn equivalent(ch: char) -> Option<char> {
    match ch {
        '\u{a0}' | '\u{2007}' | '\u{202f}' | '\u{2028}' | '\u{2029}' => Some(' '),
        '\u{ad}' | '\u{2010}' | '\u{2011}' | '\u{2212}' => Some('-'),
        _ => None,
    }
}

impl EmbeddedFont {
    /// Whether the font shows text as one byte per character.
    pub fn is_simple(&self) -> bool {
        matches!(self.encoding, Encoding::Simple { .. })
    }

    fn own(&self, ch: char) -> Option<Code> {
        match &self.encoding {
            Encoding::Simple { codes, .. } => codes.get(&ch).map(|&(code, advance)| Code {
                code: u16::from(code),
                advance: f64::from(advance),
            }),
            Encoding::Identity {
                glyphs,
                cids,
                widths,
                default,
            } => {
                let glyph = *glyphs.get(usize::try_from(u32::from(ch)).ok()?)?;
                if glyph == 0 {
                    return None;
                }
                let cid = match cids {
                    None => glyph,
                    Some(map) => *map.get(&glyph)?,
                };
                let advance = widths.get(&cid).copied().unwrap_or(*default);
                Some(Code {
                    code: cid,
                    advance: f64::from(advance),
                })
            }
        }
    }

    /// The code and advance of `ch`, or of the character it may be shown
    /// as (`equivalent`); None when the font has neither.
    pub fn code(&self, ch: char) -> Option<Code> {
        self.own(ch)
            .or_else(|| equivalent(ch).and_then(|e| self.own(e)))
    }

    /// Whether every character of `text` that is drawn (control
    /// characters, tabs among them, are not) has a code.
    pub fn covers(&self, text: &str) -> bool {
        text.chars()
            .filter(|c| !c.is_control())
            .all(|c| self.code(c).is_some())
    }

    /// `text` as the bytes a `Tj` shows: one per character for a simple
    /// font, two (big-endian CIDs) for a Type0 one. Characters without a
    /// code are left out.
    pub fn encode(&self, text: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(text.len() * 2);
        for ch in text.chars() {
            let Some(code) = self.code(ch) else {
                continue;
            };
            if self.is_simple() {
                out.push(u8::try_from(code.code).unwrap_or(0));
            } else {
                out.extend_from_slice(&code.code.to_be_bytes());
            }
        }
        out
    }

    /// Whether a widget's /DA may name the font: a simple WinAnsi font of
    /// a family `forms::appearance` does not take as symbolic, covering
    /// printable ASCII (what a fill may type) and `value`.
    fn serves_widgets(&self, value: &str) -> bool {
        let symbolic = {
            let lower = self.base_font.to_ascii_lowercase();
            lower.contains("dingbats") || lower.contains("zapf") || self.family == "symbol"
        };
        matches!(self.encoding, Encoding::Simple { win_ansi: true, .. })
            && !symbolic
            && (' '..='~').all(|c| self.own(c).is_some())
            && self.covers(value)
    }
}

impl EmbeddedFonts {
    /// The embedded fonts of `doc`; None when it embeds none that is used.
    pub fn scan(doc: &Document<'_>) -> Option<Self> {
        let mut scanner = Scanner {
            doc,
            fonts: Vec::new(),
            seen: BTreeSet::new(),
            xobjects: BTreeSet::new(),
            entries: 0,
            program_bytes: 0,
        };
        let catalog = doc.resolve_id(doc.root()).as_dict();
        let acro = catalog.and_then(|c| doc.resolve_dict(doc.dict_get(c, b"AcroForm")));
        if let Some(dr) = acro.and_then(|a| doc.resolve_dict(doc.dict_get(a, b"DR"))) {
            scanner.resources(dr, 0);
        }
        for page in info::pages(doc).iter().take(MAX_PAGES) {
            if let Some(resources) = inherited(doc, page.dict, b"Resources").as_dict() {
                scanner.resources(resources, 0);
            }
        }
        (!scanner.fonts.is_empty()).then_some(Self {
            fonts: scanner.fonts,
        })
    }

    /// A table of `fonts` (the first `MAX_FONTS`), as `scan` would make it.
    pub fn from_fonts(mut fonts: Vec<EmbeddedFont>) -> Self {
        fonts.truncate(MAX_FONTS);
        Self { fonts }
    }

    /// The fonts, in discovery order.
    pub fn fonts(&self) -> &[EmbeddedFont] {
        &self.fonts
    }

    pub fn get(&self, index: usize) -> Option<&EmbeddedFont> {
        self.fonts.get(index)
    }

    /// The first font of `font`'s family, weight and posture that covers
    /// every character of `text`.
    pub fn find(&self, font: &Font, text: &str) -> Option<usize> {
        let key = xfa_family_key(&font.family);
        if key.is_empty() {
            return None;
        }
        self.fonts.iter().position(|f| {
            f.family == key && f.bold == font.bold && f.italic == font.italic && f.covers(text)
        })
    }

    /// The font a widget showing `value` in `font` names in its /DA
    /// (`EmbeddedFont::serves_widgets`), if any.
    pub fn widget_font(&self, font: &Font, value: &str) -> Option<usize> {
        let key = xfa_family_key(&font.family);
        if key.is_empty() {
            return None;
        }
        self.fonts.iter().position(|f| {
            f.family == key
                && f.bold == font.bold
                && f.italic == font.italic
                && f.serves_widgets(value)
        })
    }

    /// The resource name the emitter registers font `index` under, in the
    /// page resources and /AcroForm /DR: `XE1`, `XE2`...
    pub fn resource_name(index: usize) -> String {
        format!("XE{}", index + 1)
    }
}

// ----------------------------------------------------------------- names

/// A family as compared: lowercased, only ASCII letters and digits kept
/// (`Times New Roman`, `TimesNewRoman` and `times-new-roman` are one).
pub fn family_key(family: &str) -> String {
    family
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// An XFA typeface's key: the first of a comma list, quotes stripped.
fn xfa_family_key(family: &str) -> String {
    let first = family.split(',').next().unwrap_or("");
    family_key(first.trim().trim_matches(|c| c == '"' || c == '\''))
}

/// The weight and posture a /BaseFont style suffix names (`Bold`,
/// `Italic`, `BoldItalicMT`, `Regular`...); None for any other suffix
/// (`Semibold`, `Light`, `Condensed`: part of the family).
fn style_of(suffix: &str) -> Option<(bool, bool)> {
    let lower = suffix.to_ascii_lowercase();
    let lower = lower
        .strip_suffix("psmt")
        .or_else(|| lower.strip_suffix("mt"))
        .unwrap_or(&lower);
    match lower {
        "" | "regular" | "roman" | "normal" | "book" | "plain" => Some((false, false)),
        "bold" | "bd" => Some((true, false)),
        "italic" | "oblique" | "it" | "ital" => Some((false, true)),
        "bolditalic" | "boldoblique" | "boldit" | "bi" => Some((true, true)),
        _ => None,
    }
}

/// A /BaseFont's family key, bold and italic: a six-capital subset tag
/// and `+` dropped; a style after `,` or the last `-` split off when it is
/// one (`style_of`); a trailing `PSMT` or `MT` dropped from the family
/// (`ArialMT` is Arial, `TimesNewRomanPSMT` Times New Roman,
/// `CourierNewPSMT` Courier New).
pub fn parse_base_font(name: &str) -> (String, bool, bool) {
    let bytes = name.as_bytes();
    let tagged =
        bytes.len() > 7 && bytes[6] == b'+' && bytes[..6].iter().all(|b| b.is_ascii_uppercase());
    let name = if tagged { &name[7..] } else { name };
    let split = name
        .split_once(',')
        .or_else(|| name.rsplit_once('-'))
        .and_then(|(family, style)| style_of(style).map(|s| (family, s)));
    let (family, (bold, italic)) = split.unwrap_or((name, (false, false)));
    let family = family
        .strip_suffix("PSMT")
        .or_else(|| family.strip_suffix("MT"))
        .filter(|f| !f.is_empty())
        .unwrap_or(family);
    (family_key(family), bold, italic)
}

// ------------------------------------------------------------- discovery

struct Scanner<'s, 'd> {
    doc: &'s Document<'d>,
    fonts: Vec<EmbeddedFont>,
    seen: BTreeSet<ObjectId>,
    xobjects: BTreeSet<ObjectId>,
    entries: usize,
    program_bytes: usize,
}

/// A font program as read.
enum Program {
    Sfnt(SfntInfo),
    /// A bare CFF program (Type1C): no cmap.
    Bare,
}

impl Scanner<'_, '_> {
    fn full(&self) -> bool {
        self.fonts.len() >= MAX_FONTS || self.entries >= MAX_FONT_ENTRIES
    }

    fn resources(&mut self, resources: &Dictionary, depth: usize) {
        let doc = self.doc;
        if let Some(fonts) = doc.resolve_dict(doc.dict_get(resources, b"Font")) {
            for (_, value) in fonts.iter() {
                if self.full() {
                    return;
                }
                self.font(value);
            }
        }
        if depth >= MAX_XOBJECT_DEPTH {
            return;
        }
        let Some(xobjects) = doc.resolve_dict(doc.dict_get(resources, b"XObject")) else {
            return;
        };
        for (_, value) in xobjects.iter() {
            if self.full() || self.xobjects.len() >= MAX_XOBJECTS {
                return;
            }
            // Form XObjects are streams: always indirect.
            let Some(id) = value.as_reference() else {
                continue;
            };
            if !self.xobjects.insert(id) {
                continue;
            }
            let Some(dict) = doc.resolve_id(id).as_dict() else {
                continue;
            };
            if doc.dict_get(dict, b"Subtype").as_name() != Some(b"Form") {
                continue;
            }
            if let Some(inner) = doc.resolve_dict(doc.dict_get(dict, b"Resources")) {
                self.resources(inner, depth + 1);
            }
        }
    }

    fn font(&mut self, value: &Object) {
        self.entries += 1;
        if let Some(id) = value.as_reference() {
            if !self.seen.insert(id) {
                return;
            }
        }
        let Some(dict) = self.doc.resolve_dict(value) else {
            return;
        };
        if let Some(mut font) = self.read(dict) {
            font.object = value.clone();
            self.fonts.push(font);
        }
    }

    fn read(&mut self, dict: &Dictionary) -> Option<EmbeddedFont> {
        let doc = self.doc;
        match doc.dict_get(dict, b"Subtype").as_name()? {
            b"Type1" | b"MMType1" | b"TrueType" => self.simple(dict),
            b"Type0" => self.type0(dict),
            _ => None,
        }
    }

    /// The decoded stream `value` names, within the program budgets.
    fn bytes(&mut self, id: ObjectId, max: usize) -> Option<Vec<u8>> {
        let room = MAX_TOTAL_PROGRAM_BYTES.saturating_sub(self.program_bytes);
        let max = max.min(room);
        if max == 0 {
            return None;
        }
        let limits = StreamLimits {
            max_decoded_bytes: max,
            max_total_decoded_bytes: max,
            ..StreamLimits::default()
        };
        let bytes = self.doc.decoded_stream(id, limits).ok()?;
        self.program_bytes = self.program_bytes.saturating_add(bytes.len());
        Some(bytes)
    }

    /// The descriptor's program: /FontFile2, or /FontFile3 of /Subtype
    /// /OpenType (an sfnt) or /Type1C (bare CFF).
    fn program(&mut self, descriptor: &Dictionary) -> Option<Program> {
        let doc = self.doc;
        if let Some(id) = descriptor
            .get(b"FontFile2".as_slice())
            .and_then(Object::as_reference)
        {
            let bytes = self.bytes(id, MAX_PROGRAM_BYTES)?;
            return SfntInfo::read(&bytes).map(Program::Sfnt);
        }
        let id = descriptor
            .get(b"FontFile3".as_slice())
            .and_then(Object::as_reference)?;
        let subtype = doc
            .stream_dict(id)
            .ok()
            .and_then(|d| doc.dict_get(d, b"Subtype").as_name().map(<[u8]>::to_vec))?;
        match subtype.as_slice() {
            b"OpenType" => {
                let bytes = self.bytes(id, MAX_PROGRAM_BYTES)?;
                SfntInfo::read(&bytes).map(Program::Sfnt)
            }
            b"Type1C" => {
                // A CFF header: major version 1, then its size.
                let bytes = self.bytes(id, MAX_PROGRAM_BYTES)?;
                (bytes.len() > 4 && bytes[0] == 1).then_some(Program::Bare)
            }
            _ => None,
        }
    }

    /// Family, weight and posture from /BaseFont and the descriptor.
    fn style(&self, base: &str, descriptor: &Dictionary) -> (String, bool, bool) {
        let doc = self.doc;
        let (family, mut bold, mut italic) = parse_base_font(base);
        let flags = doc.dict_get(descriptor, b"Flags").as_i64().unwrap_or(0);
        let weight = doc
            .dict_get(descriptor, b"FontWeight")
            .as_f64()
            .unwrap_or(0.0);
        let angle = doc
            .dict_get(descriptor, b"ItalicAngle")
            .as_f64()
            .unwrap_or(0.0);
        bold |= weight >= 600.0 || flags & (1 << 18) != 0;
        italic |= flags & (1 << 6) != 0 || (angle.is_finite() && angle != 0.0);
        (family, bold, italic)
    }

    /// Ascent and descent: the descriptor's, else the program's, else
    /// Helvetica's.
    fn extent(&self, descriptor: &Dictionary, program: Option<&SfntInfo>) -> (f64, f64) {
        let doc = self.doc;
        let good = |v: f64| v.is_finite() && v > 0.0 && v <= 2000.0;
        let ascent = doc
            .dict_get(descriptor, b"Ascent")
            .as_f64()
            .filter(|&v| good(v));
        let descent = (doc.dict_get(descriptor, b"Descent").as_f64())
            .map(|v| -v)
            .filter(|&v| good(v));
        let ascent = ascent
            .or_else(|| program.map(|p| p.ascent).filter(|&v| good(v)))
            .unwrap_or(DEFAULT_ASCENT);
        let descent = descent
            .or_else(|| program.map(|p| p.descent).filter(|&v| good(v)))
            .unwrap_or(DEFAULT_DESCENT);
        (ascent, descent)
    }

    fn simple(&mut self, dict: &Dictionary) -> Option<EmbeddedFont> {
        let doc = self.doc;
        let base = String::from_utf8_lossy(doc.dict_get(dict, b"BaseFont").as_name()?).into_owned();
        let descriptor = doc.resolve_dict(doc.dict_get(dict, b"FontDescriptor"))?;
        let program = self.program(descriptor)?;
        let (encoding, win_ansi) = simple_encoding(doc, doc.dict_get(dict, b"Encoding"))?;
        let first = doc.dict_get(dict, b"FirstChar").as_i64().unwrap_or(0);
        let widths = doc.dict_get(dict, b"Widths").as_array();
        let width_of = |code: u8| -> Option<f64> {
            let at = usize::try_from(i64::from(code) - first).ok()?;
            let width = doc.resolve(widths?.get(at)?).as_f64()?;
            width.is_finite().then_some(width)
        };
        let sfnt = match &program {
            Program::Sfnt(info) => Some(info),
            Program::Bare => None,
        };
        let mut codes = BTreeMap::new();
        for code in 0..=255u8 {
            let Some(name) = encoding.glyph_name(code) else {
                continue;
            };
            if name == b".notdef" {
                continue;
            }
            let Some(&[ch]) = registry_unicode(name) else {
                continue;
            };
            let advance = match sfnt {
                Some(info) => {
                    let glyph = simple_glyph(info, ch, code);
                    if glyph == 0 {
                        continue;
                    }
                    width_of(code).unwrap_or_else(|| info.advance(glyph))
                }
                None => match width_of(code) {
                    Some(width) if width > 0.0 => width,
                    _ => continue,
                },
            };
            codes.entry(ch).or_insert((code, advance as f32));
        }
        if codes.is_empty() {
            return None;
        }
        let (family, bold, italic) = self.style(&base, descriptor);
        let (ascent, descent) = self.extent(descriptor, sfnt);
        Some(EmbeddedFont {
            object: Object::Null,
            base_font: base,
            family,
            bold,
            italic,
            ascent,
            descent,
            encoding: Encoding::Simple { codes, win_ansi },
        })
    }

    fn type0(&mut self, dict: &Dictionary) -> Option<EmbeddedFont> {
        let doc = self.doc;
        if doc.dict_get(dict, b"Encoding").as_name() != Some(b"Identity-H") {
            return None;
        }
        let descendant =
            doc.resolve_dict(doc.dict_get(dict, b"DescendantFonts").as_array()?.first()?)?;
        if doc.dict_get(descendant, b"Subtype").as_name() != Some(b"CIDFontType2") {
            return None;
        }
        let base = doc
            .dict_get(descendant, b"BaseFont")
            .as_name()
            .or_else(|| doc.dict_get(dict, b"BaseFont").as_name())?;
        let base = String::from_utf8_lossy(base).into_owned();
        let descriptor = doc.resolve_dict(doc.dict_get(descendant, b"FontDescriptor"))?;
        let Program::Sfnt(info) = self.program(descriptor)? else {
            return None;
        };
        let cids = match descendant.get(b"CIDToGIDMap".as_slice()) {
            None => None,
            Some(value) => match doc.resolve(value) {
                Object::Null => None,
                Object::Name(name) if name.as_slice() == b"Identity" => None,
                _ => {
                    let id = value.as_reference()?;
                    let map = self.bytes(id, MAX_CID_MAP_BYTES)?;
                    let mut cids = HashMap::new();
                    for (cid, pair) in map.chunks_exact(2).enumerate() {
                        let glyph = u16::from_be_bytes([pair[0], pair[1]]);
                        if glyph != 0 {
                            cids.entry(glyph)
                                .or_insert(u16::try_from(cid).unwrap_or(u16::MAX));
                        }
                    }
                    Some(cids)
                }
            },
        };
        let widths = cid_widths(doc, doc.dict_get(descendant, b"W"));
        let default = doc
            .dict_get(descendant, b"DW")
            .as_f64()
            .filter(|w| w.is_finite())
            .unwrap_or(1000.0) as f32;
        let (family, bold, italic) = self.style(&base, descriptor);
        let (ascent, descent) = self.extent(descriptor, Some(&info));
        let glyphs = info.unicode?;
        Some(EmbeddedFont {
            object: Object::Null,
            base_font: base,
            family,
            bold,
            italic,
            ascent,
            descent,
            encoding: Encoding::Identity {
                glyphs,
                cids,
                widths,
                default,
            },
        })
    }
}

/// A simple font's encoding and whether it is the name /WinAnsiEncoding:
/// a named encoding, a dictionary's /BaseEncoding (StandardEncoding when
/// absent) with its /Differences, StandardEncoding when there is none.
/// None for anything else, or /Differences that do not read.
fn simple_encoding(doc: &Document<'_>, value: &Object) -> Option<(SimpleEncoding, bool)> {
    match value {
        Object::Null => Some((SimpleEncoding::from_named(NamedEncoding::Standard), false)),
        Object::Name(name) => {
            let named = NamedEncoding::from_pdf_name(name)?;
            Some((
                SimpleEncoding::from_named(named),
                named == NamedEncoding::WinAnsi,
            ))
        }
        Object::Dictionary(dict) => {
            let named = match doc.dict_get(dict, b"BaseEncoding").as_name() {
                Some(name) => NamedEncoding::from_pdf_name(name)?,
                None => NamedEncoding::Standard,
            };
            let base = SimpleEncoding::from_named(named);
            let encoding = match doc.dict_get(dict, b"Differences") {
                Object::Null => base,
                differences => base
                    .with_object_differences(differences, EncodingLimits::default())
                    .ok()?,
            };
            Some((encoding, false))
        }
        _ => None,
    }
}

/// The glyph a simple sfnt font draws for `code` (whose glyph name is the
/// character `ch`): its Unicode cmap at `ch`, else its (3,0) cmap at
/// 0xF000 plus the code or at the code, else its (1,0) cmap at the code;
/// 0 when none has it.
fn simple_glyph(info: &SfntInfo, ch: char, code: u8) -> u16 {
    let at = |map: &Option<Vec<u16>>, c: u32| {
        map.as_ref()
            .and_then(|m| m.get(usize::try_from(c).ok()?).copied())
            .unwrap_or(0)
    };
    let code = u32::from(code);
    [
        at(&info.unicode, u32::from(ch)),
        at(&info.symbol, 0xF000 + code),
        at(&info.symbol, code),
        at(&info.mac, code),
    ]
    .into_iter()
    .find(|&g| g != 0)
    .unwrap_or(0)
}

/// /W: `c [w1 w2 ...]` and `c_first c_last w` entries, CIDs past 0xFFFF
/// and entries past `MAX_CID_WIDTHS` left out; reading stops at the first
/// malformed entry.
fn cid_widths(doc: &Document<'_>, value: &Object) -> HashMap<u16, f32> {
    let mut out = HashMap::new();
    let Some(items) = value.as_array() else {
        return out;
    };
    let mut budget = MAX_CID_WIDTHS;
    let number = |o: &Object| doc.resolve(o).as_f64().filter(|v| v.is_finite());
    let mut i = 0;
    while i < items.len() && budget > 0 {
        let Some(first) = doc.resolve(&items[i]).as_i64() else {
            break;
        };
        match items.get(i + 1).map(|o| doc.resolve(o)) {
            Some(Object::Array(list)) => {
                for (k, w) in list.iter().enumerate() {
                    let Some(w) = number(w) else {
                        continue;
                    };
                    let Ok(cid) = u16::try_from(first.saturating_add(k as i64)) else {
                        break;
                    };
                    if budget == 0 {
                        break;
                    }
                    budget -= 1;
                    out.insert(cid, w as f32);
                }
                i += 2;
            }
            Some(last) => {
                let (Some(last), Some(w)) = (last.as_i64(), items.get(i + 2).and_then(number))
                else {
                    break;
                };
                let (Ok(first), Ok(last)) = (u16::try_from(first), u16::try_from(last.min(0xFFFF)))
                else {
                    break;
                };
                for cid in first..=last {
                    if budget == 0 {
                        break;
                    }
                    budget -= 1;
                    out.insert(cid, w as f32);
                }
                i += 3;
            }
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_font_names_split_into_family_and_style() {
        let p = |n: &str| parse_base_font(n);
        assert_eq!(p("Arial"), ("arial".into(), false, false));
        assert_eq!(p("ArialMT"), ("arial".into(), false, false));
        assert_eq!(p("ABCDEF+Arial-BoldMT"), ("arial".into(), true, false));
        assert_eq!(p("Arial,Bold"), ("arial".into(), true, false));
        assert_eq!(p("Arial,BoldItalic"), ("arial".into(), true, true));
        assert_eq!(p("Arial-BoldItalicMT"), ("arial".into(), true, true));
        assert_eq!(p("Calibri-Italic"), ("calibri".into(), false, true));
        assert_eq!(p("MyriadPro-Regular"), ("myriadpro".into(), false, false));
        assert_eq!(
            p("MyriadPro-Semibold"),
            ("myriadprosemibold".into(), false, false)
        );
        assert_eq!(
            p("TimesNewRomanPSMT"),
            ("timesnewroman".into(), false, false)
        );
        assert_eq!(
            p("TimesNewRomanPS-BoldMT"),
            ("timesnewromanps".into(), true, false)
        );
        assert_eq!(p("CourierNewPSMT"), ("couriernew".into(), false, false));
        // Not a subset tag: lowercase, or no `+`.
        assert_eq!(p("abcdef+Arial"), ("abcdefarial".into(), false, false));
        assert_eq!(xfa_family_key("Times New Roman"), "timesnewroman");
        assert_eq!(xfa_family_key("'Myriad Pro', sans-serif"), "myriadpro");
    }

    fn simple(chars: &[(char, u8, f32)], win_ansi: bool) -> EmbeddedFont {
        EmbeddedFont {
            object: Object::Null,
            base_font: "TestSans".into(),
            family: "testsans".into(),
            bold: false,
            italic: false,
            ascent: 800.0,
            descent: 200.0,
            encoding: Encoding::Simple {
                codes: chars.iter().map(|&(c, code, w)| (c, (code, w))).collect(),
                win_ansi,
            },
        }
    }

    fn font(family: &str) -> Font {
        Font {
            family: family.into(),
            ..Font::default()
        }
    }

    #[test]
    fn a_run_uses_the_font_only_when_it_covers_every_character() {
        let fonts = EmbeddedFonts {
            fonts: vec![simple(&[('A', 65, 600.0), (' ', 32, 250.0)], true)],
        };
        assert_eq!(fonts.find(&font("Test Sans"), "A A"), Some(0));
        assert_eq!(fonts.find(&font("TestSans"), "A\u{a0}A\t"), Some(0));
        assert_eq!(fonts.find(&font("TestSans"), "AB"), None);
        assert_eq!(fonts.find(&font("Arial"), "A"), None);
        let bold = Font {
            bold: true,
            ..font("TestSans")
        };
        assert_eq!(fonts.find(&bold, "A"), None);
        // A widget needs printable ASCII covered.
        assert_eq!(fonts.widget_font(&font("TestSans"), "A"), None);
        let ascii: Vec<(char, u8, f32)> =
            (0x20u8..=0x7e).map(|b| (char::from(b), b, 500.0)).collect();
        let full = EmbeddedFonts {
            fonts: vec![simple(&ascii, false), simple(&ascii, true)],
        };
        assert_eq!(full.widget_font(&font("TestSans"), "Ann"), Some(1));
        assert_eq!(full.widget_font(&font("TestSans"), "Zo\u{eb}"), None);
    }

    #[test]
    fn identity_fonts_show_two_byte_cids() {
        let mut glyphs = vec![0u16; 0x1_0000];
        glyphs[usize::from(b'A')] = 3;
        glyphs[0x4e2d] = 7;
        let mut f = simple(&[], false);
        f.encoding = Encoding::Identity {
            glyphs: glyphs.clone(),
            cids: None,
            widths: HashMap::from([(3, 550.0)]),
            default: 1000.0,
        };
        assert_eq!(f.encode("A\u{4e2d}"), vec![0, 3, 0, 7]);
        assert_eq!(f.code('A').unwrap().advance, 550.0);
        assert_eq!(f.code('\u{4e2d}').unwrap().advance, 1000.0);
        assert!(f.covers("A\u{4e2d}") && !f.covers("B"));
        // A /CIDToGIDMap: the first CID that maps to the glyph.
        f.encoding = Encoding::Identity {
            glyphs,
            cids: Some(HashMap::from([(3, 0x0102), (7, 9)])),
            widths: HashMap::new(),
            default: 500.0,
        };
        assert_eq!(f.encode("\u{4e2d}A"), vec![0, 9, 1, 2]);
    }

    /// A classic-xref PDF of `objects`, numbered from 1.
    fn pdf(objects: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn scan_reads_simple_and_identity_fonts_with_their_widths() {
        let program = crate::view::fonts::scan::test_truetype(&[
            ('A', 600),
            ('B', 500),
            ('\u{20ac}', 450),
            (' ', 250),
        ]);
        let mut stream = format!("<< /Length {} >>\nstream\n", program.len()).into_bytes();
        stream.extend_from_slice(&program);
        stream.extend_from_slice(b"\nendstream");
        let objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R /AcroForm << /DR << /Font << /A 4 0 R /C 7 0 R >> >> >> >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /B 5 0 R /A 4 0 R >> >> >>".to_vec(),
            b"<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+TestSans-Bold /FirstChar 65 /Widths [700] \
              /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [67 /Euro] >> /FontDescriptor 6 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            b"<< /Type /FontDescriptor /Flags 32 /FontFile2 8 0 R /Ascent 900 /Descent -250 >>".to_vec(),
            b"<< /Type /Font /Subtype /Type0 /BaseFont /TestSans-Identity-H /Encoding /Identity-H /DescendantFonts [9 0 R] >>".to_vec(),
            stream,
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /TestSans /FontDescriptor 10 0 R /W [1 [900]] /DW 333 >>".to_vec(),
            b"<< /Type /FontDescriptor /Flags 32 /FontFile2 8 0 R >>".to_vec(),
        ];
        let bytes = pdf(&objects);
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let fonts = EmbeddedFonts::scan(&doc).unwrap();
        assert_eq!(fonts.fonts().len(), 2);
        let simple = &fonts.fonts()[0];
        assert_eq!(
            (simple.family.as_str(), simple.bold, simple.italic),
            ("testsans", true, false)
        );
        assert_eq!(simple.ascent, 900.0);
        assert_eq!(simple.descent, 250.0);
        // /Widths where it has the code, else the program's hmtx.
        assert_eq!(
            simple.code('A').map(|c| (c.code, c.advance)),
            Some((65, 700.0))
        );
        assert_eq!(
            simple.code('B').map(|c| (c.code, c.advance)),
            Some((66, 500.0))
        );
        // /Differences put the euro at 67, before WinAnsi's 0x80.
        assert_eq!(simple.code('\u{20ac}').map(|c| c.code), Some(67));
        assert_eq!(simple.code('\u{a0}').map(|c| c.code), Some(32));
        assert_eq!(simple.code('Z'), None);
        assert_eq!(simple.encode("AB\u{20ac}"), b"ABC".to_vec());

        let cid = &fonts.fonts()[1];
        assert_eq!((cid.family.as_str(), cid.bold), ("testsans", false));
        assert_eq!(cid.ascent, 800.0);
        assert_eq!(cid.code('A').map(|c| (c.code, c.advance)), Some((1, 900.0)));
        assert_eq!(cid.code('B').map(|c| (c.code, c.advance)), Some((2, 333.0)));
        assert_eq!(cid.encode("BA"), vec![0, 2, 0, 1]);

        let bold = Font {
            bold: true,
            ..font("TestSans")
        };
        assert_eq!(fonts.find(&bold, "AB A"), Some(0));
        assert_eq!(fonts.find(&font("TestSans"), "AB"), Some(1));
        assert_eq!(fonts.find(&font("TestSans"), "AZ"), None);
        assert!(matches!(simple.object, Object::Reference(_)));
    }

    #[test]
    fn differences_give_codes_and_cid_widths_read_both_forms() {
        let bytes = pdf(&[b"<< /Type /Catalog >>".to_vec()]);
        let doc = Document::open(&bytes, Default::default()).unwrap();
        let encoding = Object::Dictionary(Dictionary::from_iter([
            (b"BaseEncoding".to_vec(), Object::name("WinAnsiEncoding")),
            (
                b"Differences".to_vec(),
                Object::Array(vec![
                    Object::Integer(65),
                    Object::name("eacute"),
                    Object::name("Euro"),
                ]),
            ),
        ]));
        let (enc, win) = simple_encoding(&doc, &encoding).unwrap();
        assert!(!win);
        assert_eq!(enc.glyph_name(65), Some(&b"eacute"[..]));
        assert_eq!(enc.glyph_name(66), Some(&b"Euro"[..]));
        assert_eq!(enc.glyph_name(67), Some(&b"C"[..]));
        let (_, win) = simple_encoding(&doc, &Object::name("WinAnsiEncoding")).unwrap();
        assert!(win);
        assert!(simple_encoding(&doc, &Object::name("Custom")).is_none());

        let w = Object::Array(vec![
            Object::Integer(1),
            Object::Array(vec![Object::Integer(500), Object::Real(250.5)]),
            Object::Integer(10),
            Object::Integer(12),
            Object::Integer(700),
        ]);
        let widths = cid_widths(&doc, &w);
        assert_eq!(widths.get(&1), Some(&500.0));
        assert_eq!(widths.get(&2), Some(&250.5));
        assert_eq!(widths.get(&11), Some(&700.0));
        assert_eq!(widths.get(&12), Some(&700.0));
        assert_eq!(widths.len(), 5);
    }
}
