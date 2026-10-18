// SPDX-License-Identifier: MIT
//! Variable text beyond WinAnsi (spec 07 sections 4 to 6): each
//! character's script, the font it is drawn in, line breaking, and the
//! fallback fonts themselves. FreeText (spec 06) and widget appearances
//! share this one layout.
//!
//! The fallback fonts are 8-bit simple fonts for Greek, Cyrillic and the
//! Latin letters WinAnsi lacks, whose code layouts are generated from the
//! Unicode Consortium mapping files (`scripts_data.rs`), and non-embedded
//! CJK CID fonts over Adobe's predefined UTF-16 CMaps. A widget embeds its
//! 8-bit fallback fonts (owner decision W1): a subset of the engine's URW
//! Nimbus Sans outlines holding the layout's glyphs, with a /ToUnicode.
use std::sync::OnceLock;

use crate::builtin_metrics::BuiltinFace;
use crate::builtin_outlines::{BuiltinOutline, BuiltinOutlineCommand};
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};
use crate::view::scan_fonts;

use super::text::{Family, Metrics, literal, win_ansi_code};
use super::write::Sink;

#[path = "scripts_data.rs"]
pub(crate) mod data;

/// The script classes of spec 07 section 4.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    Common,
    Latin,
    Greek,
    Cyrillic,
    Han,
    Kana,
    Hangul,
}

/// The class of `c` (Unicode Scripts.txt through `data::SCRIPTS`).
pub(crate) fn class(c: char) -> Class {
    let value = u32::from(c);
    let at = data::SCRIPTS.partition_point(|&(lo, _, _)| lo <= value);
    match at.checked_sub(1).map(|at| data::SCRIPTS[at]) {
        Some((_, hi, class)) if value <= hi => class,
        _ => Class::Common,
    }
}

// ------------------------------------------------------------------ CJK

/// A CJK character collection (Adobe registry) and its non-embedded font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cjk {
    Gb1,
    Cns1,
    Japan1,
    Korea1,
}

/// A CJK glyph box on the 1000-unit em: the descriptor's /Ascent and
/// /Descent, and the box a line holding CJK text is placed by.
const CJK_ASCENT: i64 = 880;
const CJK_DESCENT: i64 = -120;

impl Cjk {
    fn index(self) -> usize {
        match self {
            Self::Gb1 => 0,
            Self::Cns1 => 1,
            Self::Japan1 => 2,
            Self::Korea1 => 3,
        }
    }

    /// Resource name and /BaseFont (F5, owner decision W2): the names
    /// Adobe documents for its non-embedded CJK fonts.
    pub fn name(self) -> &'static str {
        match self {
            Self::Gb1 => "STSong-Light",
            Self::Cns1 => "MSung-Light",
            Self::Japan1 => "HeiseiMin-W3",
            Self::Korea1 => "HYSMyeongJo-Medium",
        }
    }

    fn ordering(self) -> &'static str {
        match self {
            Self::Gb1 => "GB1",
            Self::Cns1 => "CNS1",
            Self::Japan1 => "Japan1",
            Self::Korea1 => "Korea1",
        }
    }

    /// The collection's predefined UTF-16 horizontal CMap (ISO 32000-2
    /// Table 116).
    fn cmap(self) -> &'static str {
        match self {
            Self::Gb1 => "UniGB-UTF16-H",
            Self::Cns1 => "UniCNS-UTF16-H",
            Self::Japan1 => "UniJIS-UTF16-H",
            Self::Korea1 => "UniKS-UTF16-H",
        }
    }

    /// Type0 font over the UTF-16 CMap with `descendant`.
    pub fn font(self, descendant: Object) -> Dictionary {
        let base = format!("{}-{}", self.name(), self.cmap());
        let mut font = Dictionary::new();
        font.insert(b"Type".to_vec(), Object::name("Font"));
        font.insert(b"Subtype".to_vec(), Object::name("Type0"));
        font.insert(b"BaseFont".to_vec(), Object::name(base.as_bytes()));
        font.insert(b"Encoding".to_vec(), Object::name(self.cmap()));
        font.insert(b"DescendantFonts".to_vec(), Object::Array(vec![descendant]));
        font
    }

    /// CIDFontType0 dictionary: /CIDSystemInfo with the Supplement the
    /// CMap declares, /DW 1000, and half-width /W for the CIDs ASCII maps
    /// to (the layout's advances, `advance`).
    pub fn descendant_font(self, descriptor: Object) -> Dictionary {
        let (supplement, ascii) = data::CMAPS[self.index()];
        let mut info = Dictionary::new();
        info.insert(b"Registry".to_vec(), Object::string(b"Adobe".to_vec()));
        info.insert(
            b"Ordering".to_vec(),
            Object::string(self.ordering().as_bytes().to_vec()),
        );
        info.insert(
            b"Supplement".to_vec(),
            Object::Integer(i64::from(supplement)),
        );
        let mut widths = Vec::new();
        for &(first, last) in ascii {
            widths.push(Object::Integer(i64::from(first)));
            widths.push(Object::Integer(i64::from(last)));
            widths.push(Object::Integer(HALF_WIDTH as i64));
        }
        let mut font = Dictionary::new();
        font.insert(b"Type".to_vec(), Object::name("Font"));
        font.insert(b"Subtype".to_vec(), Object::name("CIDFontType0"));
        font.insert(b"BaseFont".to_vec(), Object::name(self.name()));
        font.insert(b"CIDSystemInfo".to_vec(), Object::Dictionary(info));
        font.insert(b"FontDescriptor".to_vec(), descriptor);
        font.insert(b"DW".to_vec(), Object::Integer(FULL_WIDTH as i64));
        font.insert(b"W".to_vec(), Object::Array(widths));
        font
    }

    /// The CIDFont's descriptor (ISO 32000-2 §9.8): a generic CJK face on
    /// a 1000-unit em, symbolic and, for these four Ming/Song/Mincho
    /// faces, serif.
    pub fn descriptor(self) -> Dictionary {
        let int = Object::Integer;
        let mut d = Dictionary::new();
        d.insert(b"Type".to_vec(), Object::name("FontDescriptor"));
        d.insert(b"FontName".to_vec(), Object::name(self.name()));
        d.insert(b"Flags".to_vec(), int(4 | 2));
        d.insert(
            b"FontBBox".to_vec(),
            Object::Array(vec![int(-90), int(-210), int(1090), int(960)]),
        );
        d.insert(b"ItalicAngle".to_vec(), int(0));
        d.insert(b"Ascent".to_vec(), int(CJK_ASCENT));
        d.insert(b"Descent".to_vec(), int(CJK_DESCENT));
        d.insert(b"CapHeight".to_vec(), int(700));
        d.insert(b"StemV".to_vec(), int(72));
        d
    }

    /// The advance (1/1000 em) of `c` in this font: half width for the
    /// ASCII the CMap maps to half-width CIDs, full width otherwise.
    pub(crate) fn advance(self, c: char) -> f64 {
        if (' '..='~').contains(&c) {
            HALF_WIDTH
        } else {
            FULL_WIDTH
        }
    }

    /// The glyph box of the CJK fonts, em: (top, bottom).
    pub(crate) fn glyph_box() -> (f64, f64) {
        (CJK_ASCENT as f64 / 1000.0, CJK_DESCENT as f64 / 1000.0)
    }
}

const FULL_WIDTH: f64 = 1000.0;
const HALF_WIDTH: f64 = 500.0;

/// The collection Han and Bopomofo characters take for a /Lang value
/// (raw text-string bytes; None when absent) (F4): `zh` with the script
/// `Hant` or the region TW, HK or MO is traditional Chinese, unless the
/// script is `Hans`; other `zh` simplified; `ja` Japanese; `ko` Korean;
/// anything else simplified Chinese.
pub fn han_font(lang: Option<&[u8]>) -> Cjk {
    let Some(lang) = lang else {
        return Cjk::Gb1;
    };
    let tag = decode_text_string(&lang[..lang.len().min(256)]).to_ascii_lowercase();
    let mut subtags = tag.split(['-', '_']).map(str::trim);
    match subtags.next() {
        Some("ja") => Cjk::Japan1,
        Some("ko") => Cjk::Korea1,
        Some("zh") => {
            let (mut script, mut region) = (None, None);
            // RFC 5646: extlang (3 letters), script (4 letters), region
            // (2 letters or 3 digits), then variants and extensions.
            for subtag in subtags {
                let alpha = subtag.bytes().all(|b| b.is_ascii_alphabetic());
                let digits = subtag.bytes().all(|b| b.is_ascii_digit());
                if subtag.len() == 4 && alpha && script.is_none() && region.is_none() {
                    script = Some(subtag);
                } else if region.is_none()
                    && ((subtag.len() == 2 && alpha) || (subtag.len() == 3 && digits))
                {
                    region = Some(subtag);
                } else if subtag.len() == 1 {
                    // An extension or private use: nothing after it is
                    // a script or region.
                    break;
                }
            }
            match (script, region) {
                (Some("hant"), _) => Cjk::Cns1,
                (Some("hans"), _) => Cjk::Gb1,
                (_, Some("tw" | "hk" | "mo")) => Cjk::Cns1,
                _ => Cjk::Gb1,
            }
        }
        _ => Cjk::Gb1,
    }
}

// ------------------------------------------------------------------ 8-bit fonts

/// The 8-bit fallback encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eight {
    Greek,
    Cyrillic,
    Latin,
}

impl Eight {
    fn index(self) -> usize {
        match self {
            Self::Greek => 0,
            Self::Cyrillic => 1,
            Self::Latin => 2,
        }
    }

    /// code, scalar, glyph name of the drawable upper-half codes.
    fn table(self) -> &'static [(u8, u32, &'static [u8])] {
        match self {
            Self::Greek => &data::GREEK,
            Self::Cyrillic => &data::CYRILLIC,
            Self::Latin => &data::LATIN,
        }
    }

    fn by_char(self) -> &'static [(u32, u8)] {
        match self {
            Self::Greek => &data::GREEK_BY_CHAR,
            Self::Cyrillic => &data::CYRILLIC_BY_CHAR,
            Self::Latin => &data::LATIN_BY_CHAR,
        }
    }

    /// A short ASCII tag, distinct per variant, used in cache keys and
    /// resource names.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Greek => "Grk",
            Self::Cyrillic => "Cyr",
            Self::Latin => "Ltn",
        }
    }

    /// The code of `c` in this encoding: ASCII (< 0x80) is itself; an
    /// upper-half code only when its glyph is drawable (section 6.3).
    pub(crate) fn code(self, c: char) -> Option<u8> {
        if c.is_ascii() {
            return Some(c as u8);
        }
        let value = u32::from(c);
        let table = self.by_char();
        table
            .binary_search_by_key(&value, |entry| entry.0)
            .ok()
            .map(|at| table[at].1)
    }

    /// Advances of the 256 codes in `family`, 1/1000 em; 0 for codes
    /// not drawn. ASCII codes 0x20..0x7E take the family's WinAnsi advances.
    pub fn widths(self, family: Family) -> [f64; 256] {
        *self.cached_widths(family)
    }

    pub(crate) fn cached_widths(self, family: Family) -> &'static [f64; 256] {
        static CACHE: [[OnceLock<[f64; 256]>; 6]; 3] =
            [const { [const { OnceLock::new() }; 6] }; 3];
        let column = match family {
            Family::Helvetica => 0,
            Family::HelveticaBold => 1,
            Family::Times => 2,
            Family::Courier => 3,
            Family::Symbol => 4,
            Family::ZapfDingbats => 5,
        };
        CACHE[self.index()][column].get_or_init(|| {
            let mut out = [0.0; 256];
            let ascii = Metrics::family(family);
            for code in 0x20u8..=0x7E {
                out[usize::from(code)] = ascii.advance(code);
            }
            let measured = if family.symbolic() {
                Family::Helvetica
            } else {
                family
            };
            for &(code, _, name) in self.table() {
                out[usize::from(code)] = measured.glyph_advance(name);
            }
            out
        })
    }

    /// The last upper-half code drawn.
    fn last_code(self) -> u8 {
        self.table().last().map_or(0x7E, |entry| entry.0)
    }

    /// The simple font: /Type /Font /Subtype /Type1 /BaseFont `base`,
    /// /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [...] >>
    /// naming every drawable upper-half code, /FirstChar, /LastChar,
    /// /Widths (rounded `widths`). No /ToUnicode (callers may add one).
    pub fn font(self, base: &[u8], family: Family) -> Dictionary {
        let widths = self.cached_widths(family);
        let (first, last) = (0x20u8, self.last_code());
        let mut differences = Vec::new();
        let mut next = None;
        for &(code, _, name) in self.table() {
            if next != Some(code) {
                differences.push(Object::Integer(i64::from(code)));
            }
            differences.push(Object::name(name));
            next = code.checked_add(1);
        }
        let mut encoding = Dictionary::new();
        encoding.insert(b"Type".to_vec(), Object::name("Encoding"));
        encoding.insert(b"BaseEncoding".to_vec(), Object::name("WinAnsiEncoding"));
        encoding.insert(b"Differences".to_vec(), Object::Array(differences));
        let mut font = Dictionary::new();
        font.insert(b"Type".to_vec(), Object::name("Font"));
        font.insert(b"Subtype".to_vec(), Object::name("Type1"));
        font.insert(b"BaseFont".to_vec(), Object::name(base));
        font.insert(b"FirstChar".to_vec(), Object::Integer(i64::from(first)));
        font.insert(b"LastChar".to_vec(), Object::Integer(i64::from(last)));
        font.insert(
            b"Widths".to_vec(),
            Object::Array(
                (first..=last)
                    .map(|code| Object::Integer(widths[usize::from(code)].round() as i64))
                    .collect(),
            ),
        );
        font.insert(b"Encoding".to_vec(), Object::Dictionary(encoding));
        font
    }

    /// The family an embedded fallback font draws in: the engine carries
    /// the Nimbus Sans outlines (Helvetica's metrics) in regular and bold
    /// only, so every family but Helvetica-Bold embeds the regular face.
    pub(crate) fn embedded_family(family: Family) -> Family {
        if family == Family::HelveticaBold {
            Family::HelveticaBold
        } else {
            Family::Helvetica
        }
    }

    /// The characters this font draws, by code: printable ASCII and the
    /// drawable upper half.
    fn repertoire(self) -> impl Iterator<Item = (u8, char)> {
        let ascii = (0x20u8..=0x7E).map(|code| (code, char::from(code)));
        let upper = self
            .table()
            .iter()
            .filter_map(|&(code, value, _)| Some((code, char::from_u32(value)?)));
        ascii.chain(upper)
    }

    /// Writes the embedded form of this font for `family` through `sink`
    /// (owner decision W1): a Type1 simple font whose program (FontFile3,
    /// Type1C) is the subset of the URW Nimbus Sans outlines (OFL) this
    /// layout draws, codes as in `code`, /Widths as `widths` of
    /// `embedded_family(family)`, with a /ToUnicode. When the outlines
    /// cannot give a program, the non-embedded `font` is written instead.
    pub(crate) fn embed<S: Sink>(self, sink: &mut S, family: Family) -> Result<ObjectId, S::Error> {
        let family = Self::embedded_family(family);
        let bold = family == Family::HelveticaBold;
        let outlines = scan_fonts::outlines(bold);
        let mut glyphs = Vec::new();
        for (code, c) in self.repertoire() {
            if let Some(glyph) = outlines.glyph(c) {
                glyphs.push((code, c, glyph));
            }
        }
        let style = if bold { "Bold" } else { "Regular" };
        let name = format!("{}+NimbusSans-{style}", subset_tag(self, bold));
        let ids: Vec<usize> = glyphs.iter().map(|g| g.2).collect();
        let Some((program, bbox)) = outlines.program(name.as_bytes(), &ids) else {
            return sink.object(Object::Dictionary(self.font(family.base_font(), family)));
        };
        let mut file = Dictionary::new();
        file.insert(b"Subtype".to_vec(), Object::name("Type1C"));
        let file = sink.stream(file, program)?;
        let entries: Vec<(u8, char)> = glyphs.iter().map(|g| (g.0, g.1)).collect();
        let unicode = sink.stream(Dictionary::new(), to_unicode(&entries))?;
        let face = family.face().unwrap_or(BuiltinFace::Helvetica);
        let (cap, stem) = (cap_height(face), stem_width(face, bold));
        let int = Object::Integer;
        let mut descriptor = Dictionary::new();
        descriptor.insert(b"Type".to_vec(), Object::name("FontDescriptor"));
        descriptor.insert(b"FontName".to_vec(), Object::name(name.as_bytes()));
        descriptor.insert(b"Flags".to_vec(), int(32));
        descriptor.insert(
            b"FontBBox".to_vec(),
            Object::Array(bbox.iter().map(|&v| int(i64::from(v))).collect()),
        );
        descriptor.insert(b"ItalicAngle".to_vec(), int(0));
        descriptor.insert(b"Ascent".to_vec(), int(i64::from(bbox[3].max(1))));
        descriptor.insert(b"Descent".to_vec(), int(i64::from(bbox[1].min(-1))));
        descriptor.insert(b"CapHeight".to_vec(), int(cap));
        descriptor.insert(b"StemV".to_vec(), int(stem));
        descriptor.insert(b"FontFile3".to_vec(), Object::Reference(file));
        let descriptor = sink.object(Object::Dictionary(descriptor))?;
        let widths = self.cached_widths(family);
        let (first, last) = (0x20u8, self.last_code());
        let mut differences = Vec::new();
        let mut next = None;
        for &(code, _, glyph) in &glyphs {
            if next != Some(code) {
                differences.push(int(i64::from(code)));
            }
            differences.push(Object::name(outlines.name(glyph)));
            next = code.checked_add(1);
        }
        let drawn: Vec<u8> = glyphs.iter().map(|g| g.0).collect();
        let mut encoding = Dictionary::new();
        encoding.insert(b"Type".to_vec(), Object::name("Encoding"));
        encoding.insert(b"Differences".to_vec(), Object::Array(differences));
        let mut font = Dictionary::new();
        font.insert(b"Type".to_vec(), Object::name("Font"));
        font.insert(b"Subtype".to_vec(), Object::name("Type1"));
        font.insert(b"BaseFont".to_vec(), Object::name(name.as_bytes()));
        font.insert(b"FirstChar".to_vec(), int(i64::from(first)));
        font.insert(b"LastChar".to_vec(), int(i64::from(last)));
        font.insert(
            b"Widths".to_vec(),
            Object::Array(
                (first..=last)
                    .map(|code| {
                        let width = if drawn.contains(&code) {
                            widths[usize::from(code)].round()
                        } else {
                            0.0
                        };
                        int(width as i64)
                    })
                    .collect(),
            ),
        );
        font.insert(b"Encoding".to_vec(), Object::Dictionary(encoding));
        font.insert(b"FontDescriptor".to_vec(), Object::Reference(descriptor));
        font.insert(b"ToUnicode".to_vec(), Object::Reference(unicode));
        sink.object(Object::Dictionary(font))
    }
}

/// The six-letter subset tag (ISO 32000-2 §9.6.4) of an embedded fallback
/// font: fixed per layout and weight, as its glyph set is.
fn subset_tag(eight: Eight, bold: bool) -> String {
    let mut hash: u32 = 0x811C_9DC5;
    for byte in eight.suffix().bytes().chain([u8::from(bold)]) {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    (0..6)
        .map(|i| char::from(b'A' + ((hash >> (i * 5)) % 26) as u8))
        .collect()
}

/// The top of the face's `H`, 1/1000 em.
fn cap_height(face: BuiltinFace) -> i64 {
    let (_, _, _, top) = outline_bounds(face, b"H").unwrap_or((0, 0, 0, 700));
    i64::from(top)
}

/// The width of the face's `l` stem, 1/1000 em.
fn stem_width(face: BuiltinFace, bold: bool) -> i64 {
    match outline_bounds(face, b"l") {
        Some((x0, _, x1, _)) if x1 > x0 => i64::from(x1 - x0),
        _ => {
            if bold {
                140
            } else {
                80
            }
        }
    }
}

fn outline_bounds(face: BuiltinFace, name: &[u8]) -> Option<(i32, i32, i32, i32)> {
    let outline = BuiltinOutline::lookup(face, name)?;
    let mut b: Option<(i32, i32, i32, i32)> = None;
    let mut see = |x: i32, y: i32| {
        let r = b.get_or_insert((x, y, x, y));
        *r = (r.0.min(x), r.1.min(y), r.2.max(x), r.3.max(y));
    };
    for command in outline.commands() {
        match *command {
            BuiltinOutlineCommand::MoveTo { x, y } | BuiltinOutlineCommand::LineTo { x, y } => {
                see(x, y)
            }
            BuiltinOutlineCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                see(x1, y1);
                see(x2, y2);
                see(x, y);
            }
            BuiltinOutlineCommand::Close => {}
        }
    }
    b
}

/// A ToUnicode CMap (ISO 32000-2 §9.10.3) of one-byte codes.
fn to_unicode(entries: &[(u8, char)]) -> Vec<u8> {
    let mut out = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<00> <FF>\nendcodespacerange\n",
    );
    for chunk in entries.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for &(code, c) in chunk {
            let mut units = [0u16; 2];
            let hex: String = c
                .encode_utf16(&mut units)
                .iter()
                .map(|unit| format!("{unit:04X}"))
                .collect();
            out.push_str(&format!("<{code:02X}> <{hex}>\n"));
        }
        out.push_str("endbfchar\n");
    }
    out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    out.into_bytes()
}

// ------------------------------------------------------------------ layout

/// The font a glyph is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    /// The DA font (WinAnsi).
    Latin,
    Eight(Eight),
    Cjk(Cjk),
}

/// One drawn character.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Glyph {
    pub slot: Slot,
    /// A byte for simple fonts; the Unicode scalar for a CID font.
    pub code: u32,
    /// Advance in 1/1000 of the font size.
    pub advance: f64,
    /// The character drawn (`?` when replaced).
    pub ch: char,
}

/// The fonts a text is encoded for.
pub(crate) struct Fonts<'m> {
    /// The DA font's WinAnsi advances.
    pub latin: &'m Metrics,
    /// The standard family the 8-bit fallbacks are measured in.
    pub family: Family,
    pub han: Cjk,
    /// Latin letters WinAnsi lacks may use `Eight::Latin`.
    pub latin_extended: bool,
}

/// One laid-out line: glyphs (a wrap's trailing spaces included) and the
/// width alignment uses (trailing spaces excluded), at size 1.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub glyphs: Vec<Glyph>,
    pub width: f64,
}

/// Characters of a text laid out at most.
pub(crate) const MAX_CHARS: usize = 65_536;

/// Section 4.3, step 1: dropped before layout.
fn ignorable(c: char) -> bool {
    let value = u32::from(c);
    if c == '\u{AD}' {
        return false;
    }
    if (0x80..=0x9F).contains(&value) || value == 0x2028 || value == 0x2029 {
        return true;
    }
    let at = data::IGNORABLE.partition_point(|&(lo, _)| lo <= value);
    at.checked_sub(1)
        .is_some_and(|at| value <= data::IGNORABLE[at].1)
}

/// The canonical composition of `base` and the mark `mark` when it is a
/// character some 8-bit font of section 4.2 encodes.
fn compose(base: char, mark: char) -> Option<char> {
    let key = (u32::from(base), u32::from(mark));
    let at = data::COMPOSE
        .binary_search_by_key(&key, |&(b, m, _)| (b, m))
        .ok()?;
    char::from_u32(data::COMPOSE[at].2)
}

/// `text` as layout takes it (section 4.3): ignorable characters dropped,
/// combining marks composed, a tab a space, other control characters but
/// CR and LF dropped; at most `MAX_CHARS` characters read.
fn prepare(text: &str) -> Vec<char> {
    let mut out: Vec<char> = Vec::new();
    for c in text.chars().take(MAX_CHARS) {
        if ignorable(c) {
            continue;
        }
        match c {
            '\t' => out.push(' '),
            '\r' | '\n' => out.push(c),
            c if c.is_control() => {}
            '\u{300}'..='\u{36F}' => {
                if let Some(last) = out.last_mut()
                    && let Some(composed) = compose(*last, c)
                {
                    *last = composed;
                } else {
                    out.push(c);
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// What a character's own script decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Home {
    Fixed(Slot),
    Lossy,
    /// A Common character: the gap rule decides.
    Free,
}

fn home(c: char, fonts: &Fonts<'_>) -> Home {
    let eight = |e: Eight| {
        e.code(c)
            .map_or(Home::Lossy, |_| Home::Fixed(Slot::Eight(e)))
    };
    match class(c) {
        Class::Common => Home::Free,
        Class::Latin => {
            if win_ansi_code(c).is_some() {
                Home::Fixed(Slot::Latin)
            } else if fonts.latin_extended {
                eight(Eight::Latin)
            } else {
                Home::Lossy
            }
        }
        Class::Greek => eight(Eight::Greek),
        Class::Cyrillic => eight(Eight::Cyrillic),
        Class::Han => Home::Fixed(Slot::Cjk(fonts.han)),
        Class::Kana => Home::Fixed(Slot::Cjk(Cjk::Japan1)),
        Class::Hangul => Home::Fixed(Slot::Cjk(Cjk::Korea1)),
    }
}

/// `slot` can draw `c`.
fn encodes(slot: Slot, c: char, fonts: &Fonts<'_>) -> bool {
    match slot {
        Slot::Latin => win_ansi_code(c).is_some() && !c.is_control(),
        Slot::Eight(Eight::Latin) if !fonts.latin_extended => false,
        Slot::Eight(eight) => eight.code(c).is_some() && !c.is_control(),
        Slot::Cjk(_) => true,
    }
}

/// The glyph of `c` in `slot` (which encodes it).
fn glyph(slot: Slot, c: char, fonts: &Fonts<'_>) -> Glyph {
    match slot {
        Slot::Latin => {
            let code = win_ansi_code(c).unwrap_or(b'?');
            Glyph {
                slot,
                code: u32::from(code),
                advance: fonts.latin.advance(code),
                ch: c,
            }
        }
        Slot::Eight(eight) => {
            let code = eight.code(c).unwrap_or(b'?');
            Glyph {
                slot,
                code: u32::from(code),
                advance: eight.cached_widths(fonts.family)[usize::from(code)],
                ch: c,
            }
        }
        Slot::Cjk(cjk) => Glyph {
            slot,
            code: u32::from(c),
            advance: cjk.advance(c),
            ch: c,
        },
    }
}

fn question(fonts: &Fonts<'_>) -> Glyph {
    glyph(Slot::Latin, '?', fonts)
}

/// Fonts for one line of characters (no line breaks): every classified
/// character in its script's font, and each gap of Common characters in
/// the fonts that draw it with the fewest font changes, counting the
/// changes into the classified characters on either side. Ties keep the
/// gap in the font before it, then the font after it, then the DA font.
/// A CJK font draws any Common character next to it. Returns the glyphs
/// and whether some character became `?`.
fn assign(chars: &[char], fonts: &Fonts<'_>) -> (Vec<Glyph>, bool) {
    let homes: Vec<Home> = chars.iter().map(|&c| home(c, fonts)).collect();
    let slot_of = |home: Home| match home {
        Home::Fixed(slot) => Some(slot),
        Home::Lossy => Some(Slot::Latin),
        Home::Free => None,
    };
    // How many characters of the line each 8-bit fallback draws by script.
    let mut usage = [0usize; 3];
    for home in &homes {
        if let Home::Fixed(Slot::Eight(eight)) = home {
            usage[eight.index()] += 1;
        }
    }
    let mut out = Vec::with_capacity(chars.len());
    let mut lossy = false;
    let mut i = 0;
    while i < chars.len() {
        match homes[i] {
            Home::Fixed(slot) => out.push(glyph(slot, chars[i], fonts)),
            Home::Lossy => {
                lossy = true;
                out.push(question(fonts));
            }
            Home::Free => {
                let start = i;
                while i < chars.len() && homes[i] == Home::Free {
                    i += 1;
                }
                let left = start.checked_sub(1).and_then(|at| slot_of(homes[at]));
                let right = homes.get(i).and_then(|&h| slot_of(h));
                let near = Near { left, right, usage };
                lossy |= fill_gap(&chars[start..i], near, fonts, &mut out);
                continue;
            }
        }
        i += 1;
    }
    (out, lossy)
}

/// What surrounds a gap of Common characters: the fonts of the classified
/// characters on either side, and how many characters of the line each
/// 8-bit fallback draws by script.
#[derive(Clone, Copy)]
struct Near {
    left: Option<Slot>,
    right: Option<Slot>,
    usage: [usize; 3],
}

/// Section 4.2's Common characters of `gap`.
fn fill_gap(gap: &[char], near: Near, fonts: &Fonts<'_>, out: &mut Vec<Glyph>) -> bool {
    let Near { left, right, usage } = near;
    // The 8-bit fallbacks in order of preference: first the one the line
    // already draws the most of (a symbol then joins a font the text has,
    // so the appearance needs fewer fonts and each run reads in its own
    // script's face), then the one that draws more of this gap, then the
    // one with the larger repertoire.
    let mut fallbacks = [Eight::Greek, Eight::Cyrillic, Eight::Latin];
    fallbacks.sort_by_key(|&eight| {
        let covered = gap
            .iter()
            .filter(|&&c| !c.is_ascii() && encodes(Slot::Eight(eight), c, fonts))
            .count();
        (
            std::cmp::Reverse(usage[eight.index()]),
            std::cmp::Reverse(covered),
            std::cmp::Reverse(eight.table().len()),
        )
    });
    // Candidate fonts in order of preference: the neighbours, the DA font
    // (section 4.2), then the fallbacks.
    let mut order: Vec<Slot> = Vec::with_capacity(6);
    let candidates = [left, right, Some(Slot::Latin)]
        .into_iter()
        .chain(fallbacks.map(|eight| Some(Slot::Eight(eight))));
    for slot in candidates.flatten() {
        if !order.contains(&slot) {
            order.push(slot);
        }
    }
    let cjk_ok =
        |slot: Slot| !matches!(slot, Slot::Cjk(_)) || Some(slot) == left || Some(slot) == right;
    // Viterbi over the gap: cost (font changes, preference ranks).
    const NONE: (usize, usize) = (usize::MAX, usize::MAX);
    let states = order.len();
    let mut cost = vec![NONE; states];
    let mut back: Vec<Vec<u8>> = Vec::with_capacity(gap.len());
    let mut lossy_at = vec![false; gap.len()];
    for (k, &c) in gap.iter().enumerate() {
        let allowed: Vec<bool> = order
            .iter()
            .map(|&slot| cjk_ok(slot) && encodes(slot, c, fonts))
            .collect();
        let none = !allowed.iter().any(|a| *a);
        if none {
            lossy_at[k] = true;
        }
        let mut next = vec![NONE; states];
        let mut from = vec![0u8; states];
        for s in 0..states {
            let ok = if none {
                order[s] == Slot::Latin
            } else {
                allowed[s]
            };
            if !ok {
                continue;
            }
            let (best, at) = if k == 0 {
                let change = usize::from(left.is_some_and(|l| l != order[s]));
                ((change, 0), 0)
            } else {
                let mut best = NONE;
                let mut at = 0;
                for (p, &prev) in cost.iter().enumerate() {
                    if prev == NONE {
                        continue;
                    }
                    let candidate = (prev.0 + usize::from(p != s), prev.1);
                    if candidate < best {
                        best = candidate;
                        at = p;
                    }
                }
                (best, at)
            };
            if best == NONE {
                continue;
            }
            next[s] = (best.0, best.1 + s);
            from[s] = at as u8;
        }
        cost = next;
        back.push(from);
    }
    // The end: a change into `right`.
    let mut end = 0;
    let mut best = NONE;
    for (s, &c) in cost.iter().enumerate() {
        if c == NONE {
            continue;
        }
        let total = (c.0 + usize::from(right.is_some_and(|r| r != order[s])), c.1);
        if total < best {
            best = total;
            end = s;
        }
    }
    let mut slots = vec![0usize; gap.len()];
    let mut s = end;
    for k in (0..gap.len()).rev() {
        slots[k] = s;
        s = usize::from(back[k][s]);
    }
    let mut lossy = false;
    for (k, &c) in gap.iter().enumerate() {
        if lossy_at[k] {
            lossy = true;
            out.push(question(fonts));
        } else {
            out.push(glyph(order[slots[k]], c, fonts));
        }
    }
    lossy
}

/// The text's hard lines (LF, CR and CRLF each end one), each encoded.
pub(crate) fn shape(text: &str, fonts: &Fonts<'_>) -> (Vec<Vec<Glyph>>, bool) {
    let chars = prepare(text);
    let mut lines = Vec::new();
    let mut lossy = false;
    let mut start = 0;
    let mut i = 0;
    while i <= chars.len() {
        let at_end = i == chars.len();
        if at_end || chars[i] == '\r' || chars[i] == '\n' {
            let (glyphs, l) = assign(&chars[start..i], fonts);
            lossy |= l;
            lines.push(glyphs);
            if !at_end && chars[i] == '\r' && chars.get(i + 1) == Some(&'\n') {
                i += 1;
            }
            start = i + 1;
        }
        i += 1;
    }
    (lines, lossy)
}

/// `text` as one line; true when some character became `?`. Line breaks
/// draw nothing.
pub(crate) fn encode(text: &str, fonts: &Fonts<'_>) -> (Vec<Glyph>, bool) {
    let chars: Vec<char> = prepare(text)
        .into_iter()
        .filter(|&c| c != '\r' && c != '\n')
        .collect();
    assign(&chars, fonts)
}

/// `text` broken into at most `limit` lines of at most `max_width` at
/// `size` (section 4.4); true when lossy. FreeText's entry point (spec 06);
/// widgets shape once and `wrap` at each size they try.
#[allow(dead_code)]
pub(crate) fn lines(
    text: &str,
    fonts: &Fonts<'_>,
    size: f64,
    max_width: f64,
    limit: usize,
) -> (Vec<Line>, bool) {
    let (hard, lossy) = shape(text, fonts);
    (wrap(&hard, size, max_width, limit), lossy)
}

/// Hard lines broken at `max_width` (section 4.4), at most `limit` lines.
pub(crate) fn wrap(hard: &[Vec<Glyph>], size: f64, max_width: f64, limit: usize) -> Vec<Line> {
    let mut out = Vec::new();
    // The width a line may take, in 1/1000 em.
    let room = if size > 0.0 && size.is_finite() && max_width.is_finite() {
        (max_width / size * 1000.0).max(0.0)
    } else if max_width.is_finite() && max_width <= 0.0 {
        0.0
    } else {
        f64::INFINITY
    };
    for glyphs in hard {
        if out.len() >= limit {
            break;
        }
        if glyphs.is_empty() {
            out.push(Line {
                glyphs: Vec::new(),
                width: 0.0,
            });
            continue;
        }
        let mut start = 0;
        while start < glyphs.len() && out.len() < limit {
            let mut width = 0.0;
            let mut ink = false;
            let mut after_space = None;
            let mut i = start;
            while i < glyphs.len() {
                let g = glyphs[i];
                if g.ch == ' ' {
                    width += g.advance;
                    i += 1;
                    if ink {
                        after_space = Some(i);
                    }
                    continue;
                }
                if ink && width + g.advance > room {
                    break;
                }
                width += g.advance;
                ink = true;
                i += 1;
            }
            let end = if i < glyphs.len() {
                after_space.unwrap_or(i)
            } else {
                i
            };
            let line = glyphs[start..end].to_vec();
            out.push(Line {
                width: trimmed_width(&line),
                glyphs: line,
            });
            start = end;
        }
    }
    out
}

/// A line's width without its trailing spaces, at size 1.
fn trimmed_width(glyphs: &[Glyph]) -> f64 {
    let end = glyphs
        .iter()
        .rposition(|g| g.ch != ' ')
        .map_or(0, |at| at + 1);
    glyphs[..end].iter().map(|g| g.advance).sum::<f64>() / 1000.0
}

/// A run's string operand: a literal of bytes for simple fonts (bytes
/// outside printable ASCII as octal escapes), big-endian UTF-16 hex for a
/// CID font (surrogate pairs above U+FFFF).
pub(crate) fn string(run: &[Glyph]) -> String {
    match run.first().map(|g| g.slot) {
        Some(Slot::Cjk(_)) => {
            let mut out = String::from("<");
            for g in run {
                let c = char::from_u32(g.code).unwrap_or('\u{FFFD}');
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("{unit:04X}"));
                }
            }
            out.push('>');
            out
        }
        _ => {
            let bytes: Vec<u8> = run.iter().map(|g| (g.code & 0xFF) as u8).collect();
            literal(&bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts(metrics: &Metrics) -> Fonts<'_> {
        Fonts {
            latin: metrics,
            family: Family::Helvetica,
            han: Cjk::Gb1,
            latin_extended: true,
        }
    }

    fn texts(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.glyphs.iter().map(|g| g.ch).collect())
            .collect()
    }

    #[test]
    fn breaks_end_lines_and_empty_lines_stay() {
        let m = Metrics::family(Family::Helvetica);
        let f = fonts(&m);
        let (l, _) = lines("a\r\nb\rc", &f, 12.0, 1000.0, 100);
        assert_eq!(texts(&l), ["a", "b", "c"]);
        let (l, _) = lines("a\n\nb", &f, 12.0, 1000.0, 100);
        assert_eq!(texts(&l), ["a", "", "b"]);
    }

    #[test]
    fn wraps_keep_the_space_and_long_words_break() {
        let m = Metrics::family(Family::Helvetica);
        let f = fonts(&m);
        let (l, _) = lines("one two three", &f, 10.0, 40.0, 100);
        assert_eq!(texts(&l), ["one two ", "three"]);
        assert!((l[0].width - advance_of("one two", &f)).abs() < 1e-9);
        let word = "abcdefghij".repeat(4);
        let (l, _) = lines(&word, &f, 12.0, 56.0, 100);
        assert!(l.len() > 2);
        for line in &l {
            assert!(!line.glyphs.is_empty());
            assert!(line.width * 12.0 <= 56.0 + 1e-9);
        }
        assert_eq!(texts(&l).concat(), word);
        let (l, _) = lines(&word, &f, 12.0, 1.0, 7);
        assert_eq!(l.len(), 7);
        let (l, _) = lines(&"x".repeat(70_000), &f, 1.0, 1e9, 10);
        assert_eq!(l[0].glyphs.len(), MAX_CHARS);
    }

    fn advance_of(text: &str, f: &Fonts<'_>) -> f64 {
        super::super::text::advance(&encode(text, f).0)
    }

    fn runs(glyphs: &[Glyph]) -> Vec<Slot> {
        let mut out: Vec<Slot> = Vec::new();
        for g in glyphs {
            if out.last() != Some(&g.slot) {
                out.push(g.slot);
            }
        }
        out
    }

    #[test]
    fn encoding_follows_scripts_and_gaps() {
        let m = Metrics::family(Family::Helvetica);
        let f = fonts(&m);
        let (g, lossy) = encode("nbsp\u{a0}x shy\u{ad}x", &f);
        assert!(!lossy);
        assert_eq!(runs(&g), [Slot::Latin]);
        let (g, lossy) = encode("a\u{200B}b\u{FEFF}c\u{2028}d\u{85}e\u{200D}f\u{FE0F}g", &f);
        assert!(!lossy);
        assert_eq!(g.iter().map(|g| g.ch).collect::<String>(), "abcdefg");
        let (g, lossy) = encode("q\u{301}", &f);
        assert!(lossy);
        assert_eq!(g.iter().map(|g| g.ch).collect::<String>(), "q?");
        let (g, lossy) = encode("12 Ω", &f);
        assert!(!lossy);
        assert_eq!(runs(&g), [Slot::Eight(Eight::Greek)]);
        // A symbol only some fallbacks draw joins the one the text uses.
        let (g, _) = encode("№ Жук", &f);
        assert_eq!(runs(&g), [Slot::Eight(Eight::Cyrillic)]);
        let (g, lossy) = encode("a≤b", &f);
        assert!(!lossy);
        assert_eq!(runs(&g).len(), 3);
        let (g, lossy) = encode("Привет €", &f);
        assert!(!lossy);
        let euro = g.iter().find(|g| g.ch == '€').unwrap();
        assert_eq!((euro.slot, euro.code), (Slot::Latin, 0x80));
        for text in ["日→𠀀。", "カタカナー"] {
            let (g, lossy) = encode(text, &f);
            assert!(!lossy, "{text}");
            assert_eq!(runs(&g).len(), 1, "{text}");
            assert!(matches!(g[0].slot, Slot::Cjk(_)));
        }
        let (g, lossy) = encode("e\u{301} Lo\u{301}dz\u{301}", &f);
        assert!(!lossy);
        assert_eq!(g.iter().map(|g| g.ch).collect::<String>(), "é Lódź");
        assert!(encode("aé€→", &f).1);
        let (g, _) = encode("Ўў Ђђ Љљ Ѓѓ Ќќ Ѕѕ", &f);
        assert!(g.iter().all(|g| g.ch != '?'));
    }

    #[test]
    fn strings_are_literals_or_utf16() {
        let m = Metrics::family(Family::Helvetica);
        let f = fonts(&m);
        let (g, _) = encode("日𠀀", &f);
        assert_eq!(string(&g), "<65E5D840DC00>");
        let (g, _) = encode("a(é", &f);
        assert_eq!(string(&g), "(a\\(\\351)");
    }

    #[test]
    fn tables_hold_what_the_spec_requires() {
        assert!(Eight::Cyrillic.code('Ж').is_some_and(|c| c >= 0x80));
        assert!(Eight::Greek.code('Ω').is_some_and(|c| c >= 0x80));
        assert_eq!(Eight::Cyrillic.code('▀'), None);
        for c in ('\u{100}'..='\u{17F}').chain('\u{218}'..='\u{21B}') {
            if c.is_alphabetic() && win_ansi_code(c).is_none() {
                assert!(Eight::Latin.code(c).is_some(), "{c}");
            }
        }
        for eight in [Eight::Greek, Eight::Cyrillic, Eight::Latin] {
            assert!(eight.table().len() <= 128);
            for &(code, _, _) in eight.table() {
                assert!(code >= 0x80);
                assert!(eight.widths(Family::Helvetica)[usize::from(code)] > 0.0);
            }
        }
    }

    #[test]
    fn the_embedded_subset_draws_every_code_at_the_declared_advance() {
        for bold in [false, true] {
            let family = if bold {
                Family::HelveticaBold
            } else {
                Family::Helvetica
            };
            let outlines = scan_fonts::outlines(bold);
            for eight in [Eight::Greek, Eight::Cyrillic, Eight::Latin] {
                let widths = eight.cached_widths(family);
                for (code, c) in eight.repertoire() {
                    let glyph = outlines.glyph(c);
                    assert!(glyph.is_some(), "{eight:?} {bold} {c}");
                    let advance = outlines.advance(glyph.unwrap_or(0));
                    assert_eq!(advance, widths[usize::from(code)], "{eight:?} {bold} {c}");
                }
            }
        }
    }

    #[test]
    fn han_font_reads_bcp47() {
        for (lang, want) in [
            (None, Cjk::Gb1),
            (Some("zh"), Cjk::Gb1),
            (Some("zh-CN"), Cjk::Gb1),
            (Some("zh-SG"), Cjk::Gb1),
            (Some("zh-TW"), Cjk::Cns1),
            (Some("zh-HK"), Cjk::Cns1),
            (Some("zh-MO"), Cjk::Cns1),
            (Some("zh-Hant"), Cjk::Cns1),
            (Some("zh-Hant-CN"), Cjk::Cns1),
            (Some("zh-Hans-TW"), Cjk::Gb1),
            (Some("zh-yue-HK"), Cjk::Cns1),
            (Some("ja"), Cjk::Japan1),
            (Some("ja-JP"), Cjk::Japan1),
            (Some("ko"), Cjk::Korea1),
            (Some("ko-KR"), Cjk::Korea1),
            (Some("en"), Cjk::Gb1),
            (Some("fr"), Cjk::Gb1),
            (Some(""), Cjk::Gb1),
            (Some("ZH-tw"), Cjk::Cns1),
            (Some("JA"), Cjk::Japan1),
        ] {
            assert_eq!(han_font(lang.map(str::as_bytes)), want, "{lang:?}");
        }
    }
}
