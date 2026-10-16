// SPDX-License-Identifier: AGPL-3.0-or-later
//! Text outside WinAnsi, as MuPDF 1.28's appearance synthesis draws it
//! (measured: `reference/app-forms-20261015`): Greek in ISO 8859-7 and
//! Cyrillic in KOI8-U, each a simple font of the DA font's family with
//! MuPDF's /Differences and /Widths (resources `<DA>GRK`, `<DA>CYR`), and
//! Chinese, Japanese and Korean in UTF-16 through MuPDF's Song, Mincho,
//! Batang and Ming CID fonts, every character 1000 wide. Characters none of
//! these fonts encode are drawn as `?` and the appearance is lossy (MuPDF
//! embeds a font program for them); a FreeText's Latin letters among them
//! take the Latin font `Eight::Latin` instead (`Fonts::latin_extended`,
//! design/app-accept-annots.md).
use crate::builtin_metrics::BuiltinFace;
use crate::object::{Dictionary, Object};

use super::text::{Family, Metrics, win_ansi_code};

/// The scripts MuPDF's writer tells apart (UCDN's script property) for the
/// characters its fonts encode; everything else is Common.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Script {
    Common,
    Latin,
    Greek,
    Cyrillic,
    /// Han and Bopomofo: the CID font the language picks.
    Han,
    /// Hiragana and Katakana: Mincho.
    Kana,
    Hangul,
}

fn script(c: char) -> Script {
    let u = u32::from(c);
    let within = |ranges: &[(u32, u32)]| ranges.iter().any(|&(a, b)| (a..=b).contains(&u));
    if c.is_ascii_alphabetic()
        || within(&[
            (0xAA, 0xAA),
            (0xBA, 0xBA),
            (0xC0, 0xD6),
            (0xD8, 0xF6),
            (0xF8, 0x24F),
            (0x1E00, 0x1EFF),
            (0xFF21, 0xFF3A),
            (0xFF41, 0xFF5A),
        ])
    {
        return Script::Latin;
    }
    if within(&[
        (0x370, 0x373),
        (0x375, 0x37D),
        (0x37F, 0x384),
        (0x386, 0x386),
    ]) || within(&[(0x388, 0x3FF), (0x1F00, 0x1FFF)])
    {
        return Script::Greek;
    }
    if within(&[
        (0x400, 0x52F),
        (0x1C80, 0x1C8F),
        (0x2DE0, 0x2DFF),
        (0xA640, 0xA69F),
    ]) {
        return Script::Cyrillic;
    }
    let han = [
        (0x2E80, 0x2E99),
        (0x2E9B, 0x2EF3),
        (0x2F00, 0x2FD5),
        (0x3005, 0x3005),
        (0x3007, 0x3007),
        (0x3021, 0x3029),
        (0x3038, 0x303B),
        (0x3400, 0x4DBF),
        (0x4E00, 0x9FFF),
        (0xF900, 0xFAD9),
        (0x20000, 0x323AF),
        // Bopomofo.
        (0x2EA, 0x2EB),
        (0x3105, 0x312F),
        (0x31A0, 0x31BF),
    ];
    if within(&han) {
        return Script::Han;
    }
    let kana = [
        (0x3041, 0x3096),
        (0x309D, 0x309F),
        (0x30A1, 0x30FA),
        (0x30FD, 0x30FF),
        (0x31F0, 0x31FF),
        (0x32D0, 0x32FE),
        (0x3300, 0x3357),
        (0xFF66, 0xFF6F),
        (0xFF71, 0xFF9D),
        (0x1B000, 0x1B11F),
    ];
    if within(&kana) {
        return Script::Kana;
    }
    let hangul = [
        (0x1100, 0x11FF),
        (0x302E, 0x302F),
        (0x3131, 0x318E),
        (0x3200, 0x321E),
        (0x3260, 0x327E),
        (0xA960, 0xA97C),
        (0xAC00, 0xD7A3),
        (0xD7B0, 0xD7FB),
        (0xFFA0, 0xFFDC),
    ];
    if within(&hangul) {
        return Script::Hangul;
    }
    Script::Common
}

/// MuPDF's CID fonts for Chinese, Japanese and Korean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cjk {
    Song,
    Mincho,
    Batang,
    Ming,
}

impl Cjk {
    /// The resource name and /BaseFont.
    pub fn name(self) -> &'static str {
        match self {
            Self::Song => "Song",
            Self::Mincho => "Mincho",
            Self::Batang => "Batang",
            Self::Ming => "Ming",
        }
    }

    /// MuPDF's `pdf_add_cjk_font`: a Type0 font over a CIDFontType0 with
    /// the collection's UTF-16 CMap; `descendant` names the CIDFont, whose
    /// dictionary `descendant_font` gives.
    pub fn font(self, descendant: Object) -> Dictionary {
        let encoding = match self {
            Self::Song => "UniGB-UTF16-H",
            Self::Mincho => "UniJIS-UTF16-H",
            Self::Batang => "UniKS-UTF16-H",
            Self::Ming => "UniCNS-UTF16-H",
        };
        let mut font = Dictionary::new();
        font.insert(b"Type".to_vec(), Object::name("Font"));
        font.insert(b"Subtype".to_vec(), Object::name("Type0"));
        font.insert(b"BaseFont".to_vec(), Object::name(self.name()));
        font.insert(b"Encoding".to_vec(), Object::name(encoding));
        font.insert(b"DescendantFonts".to_vec(), Object::array([descendant]));
        font
    }

    /// The CIDFont, its descriptor named by `descriptor`.
    pub fn descendant_font(self, descriptor: Object) -> Dictionary {
        let (ordering, supplement) = match self {
            Self::Song => ("GB1", 5),
            Self::Mincho => ("Japan1", 6),
            Self::Batang => ("Korea1", 2),
            Self::Ming => ("CNS1", 7),
        };
        let info = Object::dictionary([
            ("Registry", Object::String(b"Adobe".to_vec())),
            ("Ordering", Object::String(ordering.as_bytes().to_vec())),
            ("Supplement", Object::Integer(supplement)),
        ]);
        let mut font = Dictionary::new();
        font.insert(b"Type".to_vec(), Object::name("Font"));
        font.insert(b"Subtype".to_vec(), Object::name("CIDFontType0"));
        font.insert(b"BaseFont".to_vec(), Object::name(self.name()));
        font.insert(b"CIDSystemInfo".to_vec(), info);
        font.insert(b"FontDescriptor".to_vec(), descriptor);
        font
    }

    /// The CIDFont's descriptor: symbolic and serif, MuPDF's fixed metrics.
    pub fn descriptor(self) -> Dictionary {
        let bbox = [-200, -200, 1200, 1200].map(Object::Integer);
        Dictionary::from([
            (b"Type".to_vec(), Object::name("FontDescriptor")),
            (
                b"FontName".to_vec(),
                Object::String(self.name().as_bytes().to_vec()),
            ),
            (b"FontBBox".to_vec(), Object::array(bbox)),
            (b"Flags".to_vec(), Object::Integer(6)),
            (b"ItalicAngle".to_vec(), Object::Integer(0)),
            (b"Ascent".to_vec(), Object::Integer(1000)),
            (b"Descent".to_vec(), Object::Integer(-200)),
            (b"StemV".to_vec(), Object::Integer(80)),
        ])
    }
}

/// The CID font MuPDF draws Han (and Bopomofo) in for a /Lang value
/// (`fz_text_language_from_string`): Song without a language and for
/// zh-Hans or zh-CN, Ming for any other Chinese, Batang for Korean, Mincho
/// for everything else.
pub fn han_font(lang: Option<&[u8]>) -> Cjk {
    let Some(lang) = lang else {
        return Cjk::Song;
    };
    match lang {
        b"zh-Hans" | b"zh-CN" => return Cjk::Song,
        b"zh-Hant" | b"zh-HK" | b"zh-MO" | b"zh-SG" | b"zh-TW" => return Cjk::Ming,
        _ => {}
    }
    let letter = |at: usize| {
        lang.get(at)
            .filter(|b| b.is_ascii_alphabetic())
            .map(u8::to_ascii_lowercase)
    };
    match (letter(0), letter(1), letter(2)) {
        (Some(_), None, _) | (None, _, _) => Cjk::Song,
        (Some(b'z'), Some(b'h'), None) => Cjk::Ming,
        (Some(b'k'), Some(b'o'), None) => Cjk::Batang,
        _ => Cjk::Mincho,
    }
}

/// The 8-bit fallback encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eight {
    Greek,
    Cyrillic,
    /// Latin letters WinAnsi lacks (`LATIN_EXTENDED`), for FreeText only
    /// (`Fonts::latin_extended`).
    Latin,
}

impl Eight {
    fn table(self) -> &'static [(u16, &'static str, &'static str); 128] {
        match self {
            Self::Greek => &ISO8859_7,
            Self::Cyrillic => &KOI8U,
            Self::Latin => &LATIN_EXTENDED,
        }
    }

    /// The resource name suffix.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Greek => "GRK",
            Self::Cyrillic => "CYR",
            Self::Latin => "LAT",
        }
    }

    /// The code of `c`: ASCII is itself, the rest a code MuPDF names.
    fn code(self, c: char) -> Option<u8> {
        if let Ok(byte) = u8::try_from(u32::from(c))
            && byte < 0x80
        {
            return Some(byte);
        }
        let u = u16::try_from(u32::from(c)).ok()?;
        let at = (self.table().iter())
            .position(|(unicode, name, _)| *unicode == u && !name.is_empty())?;
        u8::try_from(at + 0x80).ok()
    }

    /// The advances of the 256 codes in `family`, in thousandths: the
    /// family's WinAnsi advances below 0x7F, the URW glyph's above, 0 where
    /// MuPDF names no glyph or the fonts lack it.
    pub fn widths(self, family: Family) -> [f64; 256] {
        let latin = Metrics::family(family);
        let mut widths = [0.0; 256];
        for code in 0x20..0x7Fu8 {
            widths[usize::from(code)] = latin.advance(code);
        }
        let face = match family {
            Family::Times => Some(BuiltinFace::TimesRoman),
            Family::HelveticaBold => Some(BuiltinFace::HelveticaBold),
            Family::Courier => None,
            _ => Some(BuiltinFace::Helvetica),
        };
        for (slot, (_, name, glyph)) in widths[0x80..].iter_mut().zip(self.table()) {
            if name.is_empty() || glyph.is_empty() {
                continue;
            }
            *slot = match face {
                Some(face) => face
                    .lookup_glyph(glyph.as_bytes())
                    .map_or(0.0, |advance| advance.value()),
                None => 600.0,
            };
        }
        widths
    }

    /// MuPDF's `pdf_add_simple_font` with this encoding: `base` with a
    /// WinAnsi /Differences naming the codes above 0x7F, and /Widths from
    /// the first to the last code the fonts draw.
    pub fn font(self, base: &[u8], family: Family) -> Dictionary {
        let mut differences = Vec::new();
        let mut last = None;
        for (at, (_, name, _)) in self.table().iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            let code = at + 0x80;
            if last != Some(code.wrapping_sub(1)) {
                differences.push(Object::Integer(code as i64));
            }
            differences.push(Object::name(*name));
            last = Some(code);
        }
        let encoding = Object::dictionary([
            ("BaseEncoding", Object::name("WinAnsiEncoding")),
            ("Differences", Object::Array(differences)),
        ]);
        let widths = self.widths(family);
        let drawn = |code: &usize| widths.get(*code).is_some_and(|w| *w != 0.0);
        let first = (0..256).find(drawn).unwrap_or(32);
        let last = (0..256).rev().find(drawn).unwrap_or(first).max(first);
        let list = (first..=last)
            .map(|code| super::number(widths.get(code).copied().unwrap_or(0.0).round()));
        let mut font = Dictionary::new();
        font.insert(b"Type".to_vec(), Object::name("Font"));
        font.insert(b"Subtype".to_vec(), Object::name("Type1"));
        font.insert(b"BaseFont".to_vec(), Object::name(base));
        font.insert(b"Encoding".to_vec(), encoding);
        font.insert(b"Widths".to_vec(), Object::array(list));
        font.insert(b"FirstChar".to_vec(), Object::Integer(first as i64));
        font.insert(b"LastChar".to_vec(), Object::Integer(last as i64));
        font
    }
}

/// The font a glyph is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    /// The DA font (WinAnsi).
    Latin,
    Eight(Eight),
    Cjk(Cjk),
}

/// One character, encoded: its font, its code (a byte, or a Unicode scalar
/// for a CID font) and its advance in thousandths of the size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Glyph {
    pub slot: Slot,
    pub code: u32,
    pub advance: f64,
    pub ch: char,
}

/// The fonts a text is encoded for.
pub(crate) struct Fonts<'m> {
    pub latin: &'m Metrics,
    pub family: Family,
    /// The CID font for Han.
    pub han: Cjk,
    /// A Latin letter WinAnsi lacks takes `Eight::Latin` (FreeText; a
    /// widget's appearance keeps MuPDF's fonts and is lossy instead).
    pub latin_extended: bool,
}

/// Encodes characters one at a time, the font of each chosen as MuPDF's
/// writer chooses it, and remembers whether any was replaced by `?`.
struct Encoder<'f, 'm> {
    fonts: &'f Fonts<'m>,
    greek: Option<[f64; 256]>,
    cyrillic: Option<[f64; 256]>,
    latin: Option<[f64; 256]>,
    lossy: bool,
}

impl<'f, 'm> Encoder<'f, 'm> {
    fn new(fonts: &'f Fonts<'m>) -> Self {
        Self {
            fonts,
            greek: None,
            cyrillic: None,
            latin: None,
            lossy: false,
        }
    }

    /// The font a script's characters take; None for Common.
    fn font_of(&self, s: Script) -> Option<Slot> {
        match s {
            Script::Greek => Some(Slot::Eight(Eight::Greek)),
            Script::Cyrillic => Some(Slot::Eight(Eight::Cyrillic)),
            Script::Han => Some(Slot::Cjk(self.fonts.han)),
            Script::Kana => Some(Slot::Cjk(Cjk::Mincho)),
            Script::Hangul => Some(Slot::Cjk(Cjk::Batang)),
            Script::Latin => Some(Slot::Latin),
            Script::Common => None,
        }
    }

    /// For each character, the font of the first character with a script
    /// at or after it (MuPDF's `find_initial_script` from a line's start),
    /// one more entry for the end.
    fn ahead(&self, chars: &[char]) -> Vec<Option<Slot>> {
        let mut ahead = vec![None; chars.len() + 1];
        for (at, c) in chars.iter().enumerate().rev() {
            let next = ahead.get(at + 1).copied().flatten();
            if let Some(slot) = ahead.get_mut(at) {
                *slot = self.font_of(script(*c)).or(next);
            }
        }
        ahead
    }

    /// `c` in its script's font, a Common character in `context`'s font or
    /// else the first of WinAnsi, KOI8-U and ISO 8859-7 that has it (MuPDF
    /// draws a bullet instead); `?` when none has it.
    fn glyph(&mut self, c: char, context: &mut Slot) -> Glyph {
        let own = self.font_of(script(c));
        if let Some(own) = own {
            *context = own;
        }
        let extended = self
            .fonts
            .latin_extended
            .then_some(Slot::Eight(Eight::Latin));
        let candidates = match own {
            Some(Slot::Latin) => [Some(Slot::Latin), extended, None, None],
            Some(own) => [Some(own), None, None, None],
            None => [
                Some(*context),
                Some(Slot::Latin),
                Some(Slot::Eight(Eight::Cyrillic)),
                Some(Slot::Eight(Eight::Greek)),
            ],
        };
        let found = candidates.into_iter().flatten().find_map(|slot| {
            let code = match slot {
                Slot::Latin => win_ansi_code(c).map(u32::from),
                Slot::Eight(eight) => eight.code(c).map(u32::from),
                Slot::Cjk(_) => Some(u32::from(c)),
            }?;
            Some((slot, code))
        });
        let (slot, code, ch) = match found {
            Some((slot, code)) => (slot, code, c),
            None => {
                self.lossy = true;
                (Slot::Latin, u32::from(b'?'), '?')
            }
        };
        let family = self.fonts.family;
        let advance = match slot {
            Slot::Latin => self.fonts.latin.advance(code as u8),
            Slot::Eight(eight) => {
                let cache = match eight {
                    Eight::Greek => &mut self.greek,
                    Eight::Cyrillic => &mut self.cyrillic,
                    Eight::Latin => &mut self.latin,
                };
                cache.get_or_insert_with(|| eight.widths(family))[(code & 0xFF) as usize]
            }
            Slot::Cjk(_) => 1000.0,
        };
        Glyph {
            slot,
            code,
            advance,
            ch,
        }
    }
}

/// A text encoded as one line, and whether a character none of the fonts
/// encode was replaced by `?`.
pub(crate) fn encode(text: &str, fonts: &Fonts<'_>) -> (Vec<Glyph>, bool) {
    let mut encoder = Encoder::new(fonts);
    let chars: Vec<char> = text.chars().collect();
    let first = chars.iter().find_map(|&c| encoder.font_of(script(c)));
    let mut context = first.unwrap_or(Slot::Latin);
    let glyphs = chars
        .iter()
        .map(|&c| encoder.glyph(c, &mut context))
        .collect();
    (glyphs, encoder.lossy)
}

/// One laid-out line: its glyphs (a break's trailing space included) and
/// the width alignment uses (trailing spaces left out), at size 1.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub glyphs: Vec<Glyph>,
    pub width: f64,
}

/// MuPDF's multiline layout: every CR or LF ends a line, and a line that
/// passes `max_width` (at `size`) breaks after its last space, the space
/// kept; a word wider than the line is never split. Each line's Common
/// characters before its first scripted one take the font of the next
/// scripted character of the text. `limit` caps the lines.
pub(crate) fn lines(
    text: &str,
    fonts: &Fonts<'_>,
    size: f64,
    max_width: f64,
    limit: usize,
) -> (Vec<Line>, bool) {
    let mut encoder = Encoder::new(fonts);
    let chars: Vec<char> = text.chars().collect();
    let ahead = encoder.ahead(&chars);
    let mut lines = Vec::new();
    let mut at = 0;
    while at < chars.len() && lines.len() < limit {
        let mut context = ahead.get(at).copied().flatten().unwrap_or(Slot::Latin);
        let mut glyphs: Vec<Glyph> = Vec::new();
        let mut x = 0.0;
        let mut space = None;
        let mut next = chars.len();
        for (offset, &c) in chars.get(at..).unwrap_or(&[]).iter().enumerate() {
            if c == '\r' || c == '\n' {
                next = at + offset + 1;
                break;
            }
            let glyph = encoder.glyph(c, &mut context);
            glyphs.push(glyph);
            if c == ' ' {
                space = Some(glyphs.len());
            }
            x += glyph.advance * size / 1000.0;
            if let Some(space) = space
                && x > max_width
            {
                glyphs.truncate(space);
                next = at + space;
                break;
            }
        }
        let trimmed = glyphs.len() - glyphs.iter().rev().take_while(|g| g.ch == ' ').count();
        let width = super::text::advance(glyphs.get(..trimmed).unwrap_or(&glyphs));
        lines.push(Line { glyphs, width });
        at = next;
    }
    (lines, encoder.lossy)
}

/// A run's string: a literal of bytes for a simple font, UTF-16 hex for a
/// CID font.
pub(crate) fn string(glyphs: &[Glyph]) -> String {
    match glyphs.first().map(|g| g.slot) {
        Some(Slot::Cjk(_)) => {
            let mut out = String::from("<");
            for glyph in glyphs {
                let c = char::from_u32(glyph.code).unwrap_or('?');
                for unit in c.encode_utf16(&mut [0; 2]) {
                    out.push_str(&format!("{unit:04x}"));
                }
            }
            out.push('>');
            out
        }
        _ => {
            let bytes: Vec<u8> = glyphs.iter().map(|g| g.code as u8).collect();
            super::text::literal(&bytes)
        }
    }
}

/// KOI8-U codes 0x80..=0xFF: Unicode (0 for none), MuPDF's glyph name
/// (empty when it has none) and the URW glyph whose advance it measures
/// (empty when the fonts lack it).
pub(super) static KOI8U: [(u16, &str, &str); 128] = [
    (0x2500, "", ""),
    (0x2502, "", ""),
    (0x250C, "", ""),
    (0x2510, "", ""),
    (0x2514, "", ""),
    (0x2518, "", ""),
    (0x251C, "", ""),
    (0x2524, "", ""),
    (0x252C, "", ""),
    (0x2534, "", ""),
    (0x253C, "", ""),
    (0x2580, "upblock", ""),
    (0x2584, "dnblock", ""),
    (0x2588, "block", ""),
    (0x258C, "lfblock", ""),
    (0x2590, "rtblock", ""),
    (0x2591, "ltshade", ""),
    (0x2592, "shade", ""),
    (0x2593, "dkshade", ""),
    (0x2320, "integraltop", "integraltp"),
    (0x25A0, "blacksquare", ""),
    (0x2219, "bulletoperator", "uni2219"),
    (0x221A, "radical", "radical"),
    (0x2248, "approxequal", "approxequal"),
    (0x2264, "lessequal", "lessequal"),
    (0x2265, "greaterequal", "greaterequal"),
    (0x00A0, "nbspace", "uni00A0"),
    (0x2321, "integralbottom", "integralbt"),
    (0x00B0, "degree", "degree"),
    (0x00B2, "twosuperior", "twosuperior"),
    (0x00B7, "periodcentered", "periodcentered"),
    (0x00F7, "divide", "divide"),
    (0x2550, "", ""),
    (0x2551, "", ""),
    (0x2552, "", ""),
    (0x0451, "afii10071", "afii10071"),
    (0x0454, "afii10101", "afii10101"),
    (0x2554, "", ""),
    (0x0456, "afii10103", "afii10103"),
    (0x0457, "afii10104", "afii10104"),
    (0x2557, "", ""),
    (0x2558, "", ""),
    (0x2559, "", ""),
    (0x255A, "", ""),
    (0x255B, "", ""),
    (0x0491, "afii10098", "afii10098"),
    (0x255D, "", ""),
    (0x255E, "", ""),
    (0x255F, "", ""),
    (0x2560, "", ""),
    (0x2561, "", ""),
    (0x0401, "Iocyrillic", "afii10023"),
    (0x0404, "Ecyrillic", "afii10053"),
    (0x2563, "", ""),
    (0x0406, "Icyrillic", "afii10055"),
    (0x0407, "Yicyrillic", "afii10056"),
    (0x2566, "", ""),
    (0x2567, "", ""),
    (0x2568, "", ""),
    (0x2569, "", ""),
    (0x256A, "", ""),
    (0x0490, "Gheupturncyrillic", "afii10050"),
    (0x256C, "", ""),
    (0x00A9, "copyright", "copyright"),
    (0x044E, "afii10096", "afii10096"),
    (0x0430, "acyrillic", "afii10065"),
    (0x0431, "afii10066", "afii10066"),
    (0x0446, "afii10088", "afii10088"),
    (0x0434, "afii10069", "afii10069"),
    (0x0435, "afii10070", "afii10070"),
    (0x0444, "afii10086", "afii10086"),
    (0x0433, "afii10068", "afii10068"),
    (0x0445, "afii10087", "afii10087"),
    (0x0438, "afii10074", "afii10074"),
    (0x0439, "afii10075", "afii10075"),
    (0x043A, "afii10076", "afii10076"),
    (0x043B, "afii10077", "afii10077"),
    (0x043C, "afii10078", "afii10078"),
    (0x043D, "afii10079", "afii10079"),
    (0x043E, "afii10080", "afii10080"),
    (0x043F, "afii10081", "afii10081"),
    (0x044F, "afii10097", "afii10097"),
    (0x0440, "afii10082", "afii10082"),
    (0x0441, "afii10083", "afii10083"),
    (0x0442, "afii10084", "afii10084"),
    (0x0443, "afii10085", "afii10085"),
    (0x0436, "afii10072", "afii10072"),
    (0x0432, "afii10067", "afii10067"),
    (0x044C, "afii10094", "afii10094"),
    (0x044B, "afii10093", "afii10093"),
    (0x0437, "afii10073", "afii10073"),
    (0x0448, "afii10090", "afii10090"),
    (0x044D, "afii10095", "afii10095"),
    (0x0449, "afii10091", "afii10091"),
    (0x0447, "afii10089", "afii10089"),
    (0x044A, "afii10092", "afii10092"),
    (0x042E, "IUcyrillic", "afii10048"),
    (0x0410, "Acyrillic", "afii10017"),
    (0x0411, "Becyrillic", "afii10018"),
    (0x0426, "Tsecyrillic", "afii10040"),
    (0x0414, "Decyrillic", "afii10021"),
    (0x0415, "Iecyrillic", "afii10022"),
    (0x0424, "Efcyrillic", "afii10038"),
    (0x0413, "Gecyrillic", "afii10020"),
    (0x0425, "Khacyrillic", "afii10039"),
    (0x0418, "Iicyrillic", "afii10026"),
    (0x0419, "Iishortcyrillic", "afii10027"),
    (0x041A, "Kacyrillic", "afii10028"),
    (0x041B, "Elcyrillic", "afii10029"),
    (0x041C, "Emcyrillic", "afii10030"),
    (0x041D, "Encyrillic", "afii10031"),
    (0x041E, "Ocyrillic", "afii10032"),
    (0x041F, "Pecyrillic", "afii10033"),
    (0x042F, "IAcyrillic", "afii10049"),
    (0x0420, "Ercyrillic", "afii10034"),
    (0x0421, "Escyrillic", "afii10035"),
    (0x0422, "Tecyrillic", "afii10036"),
    (0x0423, "Ucyrillic", "afii10037"),
    (0x0416, "Zhecyrillic", "afii10024"),
    (0x0412, "Vecyrillic", "afii10019"),
    (0x042C, "Softsigncyrillic", "afii10046"),
    (0x042B, "Yericyrillic", "afii10045"),
    (0x0417, "Zecyrillic", "afii10025"),
    (0x0428, "Shacyrillic", "afii10042"),
    (0x042D, "Ereversedcyrillic", "afii10047"),
    (0x0429, "Shchacyrillic", "afii10043"),
    (0x0427, "Checyrillic", "afii10041"),
    (0x042A, "Hardsigncyrillic", "afii10044"),
];

/// ISO 8859-7 codes 0x80..=0xFF: Unicode (0 for none), MuPDF's glyph name
/// (empty when it has none) and the URW glyph whose advance it measures
/// (empty when the fonts lack it).
pub(super) static ISO8859_7: [(u16, &str, &str); 128] = [
    (0x0080, "", ""),
    (0x0081, "", ""),
    (0x0082, "", ""),
    (0x0083, "", ""),
    (0x0084, "", ""),
    (0x0085, "", ""),
    (0x0086, "", ""),
    (0x0087, "", ""),
    (0x0088, "", ""),
    (0x0089, "", ""),
    (0x008A, "", ""),
    (0x008B, "", ""),
    (0x008C, "", ""),
    (0x008D, "", ""),
    (0x008E, "", ""),
    (0x008F, "", ""),
    (0x0090, "", ""),
    (0x0091, "", ""),
    (0x0092, "", ""),
    (0x0093, "", ""),
    (0x0094, "", ""),
    (0x0095, "", ""),
    (0x0096, "", ""),
    (0x0097, "", ""),
    (0x0098, "", ""),
    (0x0099, "", ""),
    (0x009A, "", ""),
    (0x009B, "", ""),
    (0x009C, "", ""),
    (0x009D, "", ""),
    (0x009E, "", ""),
    (0x009F, "", ""),
    (0x00A0, "nbspace", "uni00A0"),
    (0x2018, "quoteleft", "quoteleft"),
    (0x2019, "quoteright", "quoteright"),
    (0x00A3, "sterling", "sterling"),
    (0x20AC, "Euro", "Euro"),
    (0x20AF, "", ""),
    (0x00A6, "brokenbar", "brokenbar"),
    (0x00A7, "section", "section"),
    (0x00A8, "dieresis", "dieresis"),
    (0x00A9, "copyright", "copyright"),
    (0x037A, "ypogegrammeni", ""),
    (0x00AB, "guillemotleft", "guillemotleft"),
    (0x00AC, "logicalnot", "logicalnot"),
    (0x00AD, "sfthyphen", "uni00AD"),
    (0x0000, "", ""),
    (0x2015, "afii00208", "afii00208"),
    (0x00B0, "degree", "degree"),
    (0x00B1, "plusminus", "plusminus"),
    (0x00B2, "twosuperior", "twosuperior"),
    (0x00B3, "threesuperior", "threesuperior"),
    (0x0384, "tonos", "tonos"),
    (0x0385, "dialytikatonos", "dieresistonos"),
    (0x0386, "Alphatonos", "Alphatonos"),
    (0x00B7, "periodcentered", "periodcentered"),
    (0x0388, "Epsilontonos", "Epsilontonos"),
    (0x0389, "Etatonos", "Etatonos"),
    (0x038A, "Iotatonos", "Iotatonos"),
    (0x00BB, "guillemotright", "guillemotright"),
    (0x038C, "Omicrontonos", "Omicrontonos"),
    (0x00BD, "onehalf", "onehalf"),
    (0x038E, "Upsilontonos", "Upsilontonos"),
    (0x038F, "Omegatonos", "Omegatonos"),
    (0x0390, "iotadieresistonos", "iotadieresistonos"),
    (0x0391, "Alpha", "Alpha"),
    (0x0392, "Beta", "Beta"),
    (0x0393, "Gamma", "Gamma"),
    (0x0394, "Deltagreek", "uni0394"),
    (0x0395, "Epsilon", "Epsilon"),
    (0x0396, "Zeta", "Zeta"),
    (0x0397, "Eta", "Eta"),
    (0x0398, "Theta", "Theta"),
    (0x0399, "Iota", "Iota"),
    (0x039A, "Kappa", "Kappa"),
    (0x039B, "Lambda", "Lambda"),
    (0x039C, "Mu", "Mu"),
    (0x039D, "Nu", "Nu"),
    (0x039E, "Xi", "Xi"),
    (0x039F, "Omicron", "Omicron"),
    (0x03A0, "Pi", "Pi"),
    (0x03A1, "Rho", "Rho"),
    (0x0000, "", ""),
    (0x03A3, "Sigma", "Sigma"),
    (0x03A4, "Tau", "Tau"),
    (0x03A5, "Upsilon", "Upsilon"),
    (0x03A6, "Phi", "Phi"),
    (0x03A7, "Chi", "Chi"),
    (0x03A8, "Psi", "Psi"),
    (0x03A9, "Omegagreek", "uni03A9"),
    (0x03AA, "Iotadieresis", "Iotadieresis"),
    (0x03AB, "Upsilondieresis", "Upsilondieresis"),
    (0x03AC, "alphatonos", "alphatonos"),
    (0x03AD, "epsilontonos", "epsilontonos"),
    (0x03AE, "etatonos", "etatonos"),
    (0x03AF, "iotatonos", "iotatonos"),
    (0x03B0, "upsilondieresistonos", "upsilondieresistonos"),
    (0x03B1, "alpha", "alpha"),
    (0x03B2, "beta", "beta"),
    (0x03B3, "gamma", "gamma"),
    (0x03B4, "delta", "delta"),
    (0x03B5, "epsilon", "epsilon"),
    (0x03B6, "zeta", "zeta"),
    (0x03B7, "eta", "eta"),
    (0x03B8, "theta", "theta"),
    (0x03B9, "iota", "iota"),
    (0x03BA, "kappa", "kappa"),
    (0x03BB, "lambda", "lambda"),
    (0x03BC, "mugreek", "uni03BC"),
    (0x03BD, "nu", "nu"),
    (0x03BE, "xi", "xi"),
    (0x03BF, "omicron", "omicron"),
    (0x03C0, "pi", "pi"),
    (0x03C1, "rho", "rho"),
    (0x03C2, "sigma1", "uni03C2"),
    (0x03C3, "sigma", "sigma"),
    (0x03C4, "tau", "tau"),
    (0x03C5, "upsilon", "upsilon"),
    (0x03C6, "phi", "phi"),
    (0x03C7, "chi", "chi"),
    (0x03C8, "psi", "psi"),
    (0x03C9, "omega", "omega"),
    (0x03CA, "iotadieresis", "iotadieresis"),
    (0x03CB, "upsilondieresis", "upsilondieresis"),
    (0x03CC, "omicrontonos", "omicrontonos"),
    (0x03CD, "upsilontonos", "upsilontonos"),
    (0x03CE, "omegatonos", "omegatonos"),
    (0x0000, "", ""),
];

/// The Latin font's codes 0x80..=0xFF (none of MuPDF's: where it embeds a
/// font program for Latin letters WinAnsi lacks, the app names the glyphs
/// of the base 14 font): Latin Extended-A but the seven WinAnsi has, then
/// S and T with comma below. Unicode (0 for none), the Adobe glyph name and
/// the URW glyph whose advance it measures (the same name).
pub(super) static LATIN_EXTENDED: [(u16, &str, &str); 128] = [
    (0x0100, "Amacron", "Amacron"),
    (0x0101, "amacron", "amacron"),
    (0x0102, "Abreve", "Abreve"),
    (0x0103, "abreve", "abreve"),
    (0x0104, "Aogonek", "Aogonek"),
    (0x0105, "aogonek", "aogonek"),
    (0x0106, "Cacute", "Cacute"),
    (0x0107, "cacute", "cacute"),
    (0x0108, "Ccircumflex", "Ccircumflex"),
    (0x0109, "ccircumflex", "ccircumflex"),
    (0x010A, "Cdotaccent", "Cdotaccent"),
    (0x010B, "cdotaccent", "cdotaccent"),
    (0x010C, "Ccaron", "Ccaron"),
    (0x010D, "ccaron", "ccaron"),
    (0x010E, "Dcaron", "Dcaron"),
    (0x010F, "dcaron", "dcaron"),
    (0x0110, "Dcroat", "Dcroat"),
    (0x0111, "dcroat", "dcroat"),
    (0x0112, "Emacron", "Emacron"),
    (0x0113, "emacron", "emacron"),
    (0x0114, "Ebreve", "Ebreve"),
    (0x0115, "ebreve", "ebreve"),
    (0x0116, "Edotaccent", "Edotaccent"),
    (0x0117, "edotaccent", "edotaccent"),
    (0x0118, "Eogonek", "Eogonek"),
    (0x0119, "eogonek", "eogonek"),
    (0x011A, "Ecaron", "Ecaron"),
    (0x011B, "ecaron", "ecaron"),
    (0x011C, "Gcircumflex", "Gcircumflex"),
    (0x011D, "gcircumflex", "gcircumflex"),
    (0x011E, "Gbreve", "Gbreve"),
    (0x011F, "gbreve", "gbreve"),
    (0x0120, "Gdotaccent", "Gdotaccent"),
    (0x0121, "gdotaccent", "gdotaccent"),
    (0x0122, "Gcommaaccent", "Gcommaaccent"),
    (0x0123, "gcommaaccent", "gcommaaccent"),
    (0x0124, "Hcircumflex", "Hcircumflex"),
    (0x0125, "hcircumflex", "hcircumflex"),
    (0x0126, "Hbar", "Hbar"),
    (0x0127, "hbar", "hbar"),
    (0x0128, "Itilde", "Itilde"),
    (0x0129, "itilde", "itilde"),
    (0x012A, "Imacron", "Imacron"),
    (0x012B, "imacron", "imacron"),
    (0x012C, "Ibreve", "Ibreve"),
    (0x012D, "ibreve", "ibreve"),
    (0x012E, "Iogonek", "Iogonek"),
    (0x012F, "iogonek", "iogonek"),
    (0x0130, "Idotaccent", "Idotaccent"),
    (0x0131, "dotlessi", "dotlessi"),
    (0x0132, "IJ", "IJ"),
    (0x0133, "ij", "ij"),
    (0x0134, "Jcircumflex", "Jcircumflex"),
    (0x0135, "jcircumflex", "jcircumflex"),
    (0x0136, "Kcommaaccent", "Kcommaaccent"),
    (0x0137, "kcommaaccent", "kcommaaccent"),
    (0x0138, "kgreenlandic", "kgreenlandic"),
    (0x0139, "Lacute", "Lacute"),
    (0x013A, "lacute", "lacute"),
    (0x013B, "Lcommaaccent", "Lcommaaccent"),
    (0x013C, "lcommaaccent", "lcommaaccent"),
    (0x013D, "Lcaron", "Lcaron"),
    (0x013E, "lcaron", "lcaron"),
    (0x013F, "Ldot", "Ldot"),
    (0x0140, "ldot", "ldot"),
    (0x0141, "Lslash", "Lslash"),
    (0x0142, "lslash", "lslash"),
    (0x0143, "Nacute", "Nacute"),
    (0x0144, "nacute", "nacute"),
    (0x0145, "Ncommaaccent", "Ncommaaccent"),
    (0x0146, "ncommaaccent", "ncommaaccent"),
    (0x0147, "Ncaron", "Ncaron"),
    (0x0148, "ncaron", "ncaron"),
    (0x0149, "napostrophe", "napostrophe"),
    (0x014A, "Eng", "Eng"),
    (0x014B, "eng", "eng"),
    (0x014C, "Omacron", "Omacron"),
    (0x014D, "omacron", "omacron"),
    (0x014E, "Obreve", "Obreve"),
    (0x014F, "obreve", "obreve"),
    (0x0150, "Ohungarumlaut", "Ohungarumlaut"),
    (0x0151, "ohungarumlaut", "ohungarumlaut"),
    (0x0154, "Racute", "Racute"),
    (0x0155, "racute", "racute"),
    (0x0156, "Rcommaaccent", "Rcommaaccent"),
    (0x0157, "rcommaaccent", "rcommaaccent"),
    (0x0158, "Rcaron", "Rcaron"),
    (0x0159, "rcaron", "rcaron"),
    (0x015A, "Sacute", "Sacute"),
    (0x015B, "sacute", "sacute"),
    (0x015C, "Scircumflex", "Scircumflex"),
    (0x015D, "scircumflex", "scircumflex"),
    (0x015E, "Scedilla", "Scedilla"),
    (0x015F, "scedilla", "scedilla"),
    (0x0162, "Tcommaaccent", "Tcommaaccent"),
    (0x0163, "tcommaaccent", "tcommaaccent"),
    (0x0164, "Tcaron", "Tcaron"),
    (0x0165, "tcaron", "tcaron"),
    (0x0166, "Tbar", "Tbar"),
    (0x0167, "tbar", "tbar"),
    (0x0168, "Utilde", "Utilde"),
    (0x0169, "utilde", "utilde"),
    (0x016A, "Umacron", "Umacron"),
    (0x016B, "umacron", "umacron"),
    (0x016C, "Ubreve", "Ubreve"),
    (0x016D, "ubreve", "ubreve"),
    (0x016E, "Uring", "Uring"),
    (0x016F, "uring", "uring"),
    (0x0170, "Uhungarumlaut", "Uhungarumlaut"),
    (0x0171, "uhungarumlaut", "uhungarumlaut"),
    (0x0172, "Uogonek", "Uogonek"),
    (0x0173, "uogonek", "uogonek"),
    (0x0174, "Wcircumflex", "Wcircumflex"),
    (0x0175, "wcircumflex", "wcircumflex"),
    (0x0176, "Ycircumflex", "Ycircumflex"),
    (0x0177, "ycircumflex", "ycircumflex"),
    (0x0179, "Zacute", "Zacute"),
    (0x017A, "zacute", "zacute"),
    (0x017B, "Zdotaccent", "Zdotaccent"),
    (0x017C, "zdotaccent", "zdotaccent"),
    (0x017F, "longs", "longs"),
    (0x0218, "Scommaaccent", "Scommaaccent"),
    (0x0219, "scommaaccent", "scommaaccent"),
    (0x021A, "uni021A", "uni021A"),
    (0x021B, "uni021B", "uni021B"),
    (0, "", ""),
    (0, "", ""),
    (0, "", ""),
];
