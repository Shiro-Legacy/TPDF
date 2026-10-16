// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Unicode of shown character codes as MuPDF 1.28 derives it for
//! structured text (pdf-font.c, pdf-type3.c, pdf-unicode.c, pdf-op-run.c):
//! ToUnicode, a CID collection's UCS2 CMap or TrueType-UCS2 for composite
//! fonts; for simple fonts the glyph names of the encoding, else of the font
//! program's built-in encoding (a TrueType program's `post` names, else
//! WinAnsi names; MuPDF's Symbol, ZapfDingbats or Standard faces when the
//! program is not embedded); and the reading direction structured text gives
//! each character. Measured against MuPDF 1.28.1; see
//! `tools/probe-text-unicode.mjs` and `design/app-corpus-text.md`.
use crate::simple_encoding::{NamedEncoding, SimpleEncoding};
use hayro_interpret::hayro_cmap::{self, BfString, CMap, CMapName, CidFamily, WritingMode};
use hayro_syntax::object::dict::keys::{
    BASE_ENCODING, BASE_FONT, CIDSYSTEMINFO, DESCENDANT_FONTS, DIFFERENCES, ENCODING, FLAGS,
    FONT_DESC, ORDERING, REGISTRY, SUBTYPE, TO_UNICODE, TYPE0, TYPE3,
};
use hayro_syntax::object::{Array, Dict, Name, Object, Stream, String as PdfString};
use std::collections::HashMap;
use std::rc::Rc;

#[path = "text_unicode_data.rs"]
mod data;

const FFFD: char = '\u{fffd}';

/// MuPDF's `fz_unicode_from_glyph_name`: the part before the first '.', the
/// TeX ligatures f_f, f_i, f_l, f_f_i, f_f_l, else the part before the first
/// '_'; then MuPDF's glyph list, `uniXXXX`, `uX...` (hexadecimal) and `aN...`
/// (decimal, two digits or more), each number read whole as C's `strtol`
/// reads it. Unlike MuPDF, values above U+FFFF are not cut to 16 bits.
pub(super) fn glyph_name(name: &[u8]) -> char {
    let name = name.split(|&b| b == b'.').next().unwrap_or_default();
    let ligature = match name {
        b"f_f" => Some('\u{fb00}'),
        b"f_i" => Some('\u{fb01}'),
        b"f_l" => Some('\u{fb02}'),
        b"f_f_i" => Some('\u{fb03}'),
        b"f_f_l" => Some('\u{fb04}'),
        _ => None,
    };
    if let Some(c) = ligature {
        return c;
    }
    let name = name.split(|&b| b == b'_').next().unwrap_or_default();
    if let Ok(at) = data::GLYPHS.binary_search_by(|(n, _)| n.cmp(&name)) {
        return char::from_u32(data::GLYPHS[at].1).unwrap_or(FFFD);
    }
    let number = if let (Some(digits), 7) = (name.strip_prefix(b"uni"), name.len()) {
        strtol(digits, 16)
    } else if let Some(digits) = name.strip_prefix(b"u") {
        strtol(digits, 16)
    } else if let (Some(digits), true) = (name.strip_prefix(b"a"), name.len() >= 3) {
        strtol(digits, 10)
    } else {
        None
    };
    number
        .filter(|v| (1..=0x10ffff).contains(v))
        .and_then(|v| char::from_u32(v as u32))
        .unwrap_or(FFFD)
}

/// C `strtol` that must read the whole input: leading white space, a sign,
/// a `0x` prefix in base 16, at least one digit.
fn strtol(text: &[u8], radix: u32) -> Option<i64> {
    let mut rest = text;
    while let [b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r', tail @ ..] = rest {
        rest = tail;
    }
    let negative = rest.first() == Some(&b'-');
    if let [b'+' | b'-', tail @ ..] = rest {
        rest = tail;
    }
    if radix == 16
        && let [b'0', b'x' | b'X', tail @ ..] = rest
        && tail.first().is_some_and(u8::is_ascii_hexdigit)
    {
        rest = tail;
    }
    if rest.is_empty() {
        return None;
    }
    let mut value: i64 = 0;
    for &b in rest {
        let digit = char::from(b).to_digit(radix)?;
        value = value
            .saturating_mul(i64::from(radix))
            .saturating_add(i64::from(digit));
    }
    Some(if negative { -value } else { value })
}

/// The direction structured text reads `c` in (MuPDF's stext, measured):
/// right to left for the R and AL classes, `None` (that of the character
/// before) for AN, CS, NSM, BN, B, S, WS and ON, else left to right.
pub(super) fn direction(c: char) -> Option<bool> {
    let v = c as u32;
    let at = data::BIDI.binary_search_by(|&(lo, hi, _)| {
        if v < lo {
            std::cmp::Ordering::Greater
        } else if v > hi {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    });
    match at {
        Ok(i) => (data::BIDI[i].2 == 1).then_some(true),
        Err(_) => Some(false),
    }
}

/// Appends the text structured text gives `c`: presentation forms and
/// ligatures become their compatibility characters (U+FEFF none), as in
/// MuPDF; everything else is itself.
pub(super) fn expand(c: char, out: &mut Vec<char>) {
    let code = c as u32;
    if (0xfb00..=0xfeff).contains(&code)
        && let Ok(at) = data::FORMS.binary_search_by_key(&code, |(c, _)| *c)
    {
        out.extend(data::FORMS[at].1.chars());
        return;
    }
    out.push(c);
}

/// A single character a ToUnicode-style map gives: white-space controls read
/// as a space, other controls as no mapping (MuPDF then falls back).
fn mapped_char(c: char) -> Option<char> {
    match c {
        '\u{8}'..='\u{d}' => Some(' '),
        '\0'..='\u{7}' | '\u{e}'..='\u{1f}' | '\u{7f}'..='\u{9f}' => None,
        _ => Some(c),
    }
}

/// `c`, or for a control character what MuPDF reads for it: a space for
/// the white-space controls, else U+FFFD.
pub(super) fn control_free(c: char) -> char {
    mapped_char(c).unwrap_or(FFFD)
}

fn mapped(value: Option<BfString>) -> Option<Vec<char>> {
    match value? {
        BfString::Char(c) => mapped_char(c).map(|c| vec![c]),
        BfString::String(s) if s.is_empty() => None,
        BfString::String(s) => Some(
            s.chars()
                .map(|c| if c == '\0' { FFFD } else { c })
                .collect(),
        ),
    }
}

/// Predefined CMaps parsed so far, by name, shared by the fonts of a text
/// reader (a CJK collection's CMaps are about a megabyte each).
pub(super) type CMaps = HashMap<Vec<u8>, Option<Rc<CMap>>>;

fn load_cmap(name: &[u8], cmaps: &mut CMaps) -> Option<Rc<CMap>> {
    cmaps
        .entry(name.to_vec())
        .or_insert_with(|| {
            let cmap = match CMapName::from_bytes(name) {
                CMapName::IdentityH => CMap::identity_h(),
                CMapName::IdentityV => CMap::identity_v(),
                name => CMap::parse(hayro_cmap::load_embedded(name)?, hayro_cmap::load_embedded)?,
            };
            Some(Rc::new(cmap))
        })
        .clone()
}

fn parse_cmap(stream: &Stream<'_>) -> Option<CMap> {
    CMap::parse(&stream.decoded().ok()?, hayro_cmap::load_embedded)
}

/// A ToUnicode stream as MuPDF reads it: hayro's parse, else the mapping
/// blocks alone (MuPDF skips any other token, such as a name cut short by
/// a stray `>`, and fails only on a malformed block).
fn parse_to_unicode(stream: &Stream<'_>) -> Option<CMap> {
    let data = stream.decoded().ok()?;
    CMap::parse(&data, hayro_cmap::load_embedded)
        .or_else(|| CMap::parse(&mapping_blocks(&data)?, |_| None))
}

/// A token of MuPDF's lexer, as a CMap block needs it.
#[derive(PartialEq)]
enum Token<'a> {
    String(Vec<u8>),
    Number(&'a [u8]),
    Keyword(&'a [u8]),
    Open,
    Close,
    Other,
}

fn is_white(b: u8) -> bool {
    matches!(b, 0 | b'\t' | b'\n' | 0x0c | b'\r' | b' ')
}

fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

/// The next token of `data` from `at` (MuPDF's `pdf_lex`, leniently).
fn lex<'a>(data: &'a [u8], at: &mut usize) -> Option<Token<'a>> {
    while *at < data.len() {
        let b = data[*at];
        if is_white(b) {
            *at += 1;
        } else if b == b'%' {
            while *at < data.len() && !matches!(data[*at], b'\r' | b'\n') {
                *at += 1;
            }
        } else {
            break;
        }
    }
    let start = *at;
    let b = *data.get(start)?;
    *at += 1;
    let word = |at: &mut usize| {
        while *at < data.len() && !is_white(data[*at]) && !is_delimiter(data[*at]) {
            *at += 1;
        }
    };
    Some(match b {
        b'[' => Token::Open,
        b']' => Token::Close,
        b'<' if data.get(*at) == Some(&b'<') => {
            *at += 1;
            Token::Other
        }
        b'<' => {
            let mut digits = Vec::new();
            while let Some(&c) = data.get(*at) {
                *at += 1;
                match c {
                    b'>' => break,
                    c if c.is_ascii_hexdigit() => digits.push(c),
                    _ => {}
                }
            }
            if digits.len() % 2 == 1 {
                digits.push(b'0');
            }
            let value = |c: u8| char::from(c).to_digit(16).unwrap_or(0) as u8;
            Token::String(
                digits
                    .chunks(2)
                    .map(|p| value(p[0]) << 4 | value(p[1]))
                    .collect(),
            )
        }
        b'(' => {
            let mut out = Vec::new();
            let mut depth = 1u32;
            while let Some(&c) = data.get(*at) {
                *at += 1;
                match c {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    b'\\' => {
                        let Some(&e) = data.get(*at) else { break };
                        *at += 1;
                        let c = match e {
                            b'n' => b'\n',
                            b'r' => b'\r',
                            b't' => b'\t',
                            b'b' => 8,
                            b'f' => 12,
                            b'0'..=b'7' => {
                                let mut v = u32::from(e - b'0');
                                for _ in 0..2 {
                                    match data.get(*at) {
                                        Some(&d @ b'0'..=b'7') => {
                                            v = v * 8 + u32::from(d - b'0');
                                            *at += 1;
                                        }
                                        _ => break,
                                    }
                                }
                                v as u8
                            }
                            b'\r' | b'\n' => {
                                if e == b'\r' && data.get(*at) == Some(&b'\n') {
                                    *at += 1;
                                }
                                continue;
                            }
                            e => e,
                        };
                        out.push(c);
                        continue;
                    }
                    _ => {}
                }
                out.push(c);
            }
            Token::String(out)
        }
        b'/' => {
            word(at);
            Token::Other
        }
        b'+' | b'-' | b'.' | b'0'..=b'9' => {
            word(at);
            Token::Number(&data[start..*at])
        }
        c if is_delimiter(c) => Token::Other,
        _ => {
            word(at);
            Token::Keyword(&data[start..*at])
        }
    })
}

/// The mapping blocks of a CMap rewritten as hayro parses them, or `None`
/// where MuPDF fails the CMap (a block with an unexpected token).
fn mapping_blocks(data: &[u8]) -> Option<Vec<u8>> {
    // Begin keyword, end keyword, and each entry's shape: c code string,
    // s destination string, n CID, a string or an array of strings.
    const BLOCKS: [(&[u8], &[u8], &[u8]); 7] = [
        (b"begincodespacerange", b"endcodespacerange", b"cc"),
        (b"beginbfchar", b"endbfchar", b"cs"),
        (b"beginbfrange", b"endbfrange", b"cca"),
        (b"begincidchar", b"endcidchar", b"cn"),
        (b"begincidrange", b"endcidrange", b"ccn"),
        (b"beginnotdefchar", b"endnotdefchar", b"cn"),
        (b"beginnotdefrange", b"endnotdefrange", b"ccn"),
    ];
    let hex = |out: &mut Vec<u8>, bytes: &[u8]| {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        out.push(b'<');
        for &b in bytes {
            out.extend([DIGITS[usize::from(b >> 4)], DIGITS[usize::from(b & 15)]]);
        }
        out.extend_from_slice(b"> ");
    };
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(token) = lex(data, &mut at) {
        let Token::Keyword(word) = token else {
            continue;
        };
        let Some(&(begin, end, shape)) = BLOCKS.iter().find(|b| b.0 == word) else {
            continue;
        };
        out.extend_from_slice(begin);
        out.push(b'\n');
        'entries: loop {
            for (i, &kind) in shape.iter().enumerate() {
                let token = lex(data, &mut at)?;
                match (kind, token) {
                    (_, Token::Keyword(w)) if i == 0 && w == end => break 'entries,
                    (b'c' | b's', Token::String(bytes)) => hex(&mut out, &bytes),
                    (b'n', Token::Number(n)) => {
                        out.extend_from_slice(n);
                        out.push(b' ');
                    }
                    (b'a', Token::String(bytes)) => hex(&mut out, &bytes),
                    (b'a', Token::Open) => {
                        out.push(b'[');
                        loop {
                            match lex(data, &mut at)? {
                                Token::Close => break,
                                Token::String(bytes) => hex(&mut out, &bytes),
                                _ => return None,
                            }
                        }
                        out.extend_from_slice(b"] ");
                    }
                    _ => return None,
                }
            }
            out.push(b'\n');
        }
        out.extend_from_slice(end);
        out.push(b'\n');
    }
    Some(out)
}

/// The CID a code maps to, by any byte length, as hayro reads it.
fn cid_of(cmap: &CMap, code: u32) -> Option<u32> {
    (1..=4).find_map(|len| cmap.lookup_cid_code(code, len))
}

enum ToUnicode {
    None,
    /// A ToUnicode stream, by code.
    Map(Rc<CMap>),
    /// A predefined CMap named as ToUnicode: MuPDF reads its CIDs as Unicode.
    Named(Rc<CMap>),
    /// A ToUnicode stream that does not parse: no text, and no fallback.
    Broken,
}

/// What a composite font without ToUnicode reads its CIDs with.
enum Collection {
    None,
    Ucs2(Rc<CMap>),
    /// MuPDF's TrueType-UCS2 (the Macintosh glyph order), for a font whose
    /// program is not embedded and whose encoding is Identity.
    TrueType,
}

/// The Unicode MuPDF gives the codes of one font.
pub(super) struct FontText {
    /// A composite font's encoding: it splits shown bytes into codes.
    encoding: Option<Rc<CMap>>,
    /// One-byte glyphs read as the encoding's codes (MuPDF's cp936 reading of
    /// S22PDF's Chinese fonts).
    regroup: bool,
    to_unicode: ToUnicode,
    collection: Collection,
    /// A simple font's text by code; `None` leaves hayro's reading (codes of
    /// an embedded Type1 or CFF program's built-in encoding).
    simple: Option<Box<[Option<char>; 256]>>,
}

/// One code of shown text: the code, its CID and how many one-byte glyphs it spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Code {
    pub code: u32,
    pub cid: u32,
    pub bytes: u8,
}

impl FontText {
    /// `program` is the decoded font program of the font (a composite
    /// font's descendant), when it has one.
    pub(super) fn new(dict: &Dict<'_>, program: Option<&[u8]>, cmaps: &mut CMaps) -> Self {
        let subtype = dict.get::<Name<'_>>(SUBTYPE);
        let to_unicode = match dict.get::<Object<'_>>(TO_UNICODE) {
            Some(Object::Stream(s)) => {
                parse_to_unicode(&s).map_or(ToUnicode::Broken, |m| ToUnicode::Map(Rc::new(m)))
            }
            Some(Object::Name(n)) => {
                load_cmap(n.as_ref(), cmaps).map_or(ToUnicode::None, ToUnicode::Named)
            }
            _ => ToUnicode::None,
        };
        if subtype.as_ref().is_some_and(|n| n.as_ref() == TYPE0) {
            return Self::composite(dict, to_unicode, program.is_some(), cmaps);
        }
        if subtype.as_ref().is_some_and(|n| n.as_ref() == TYPE3) {
            return Self {
                encoding: None,
                regroup: false,
                to_unicode,
                collection: Collection::None,
                simple: Some(type3_table(dict)),
            };
        }
        if cp936_font(dict) {
            // MuPDF reads these as GBK-EUC-H, and their CIDs with Adobe-GB1-UCS2.
            return Self {
                encoding: load_cmap(b"GBK-EUC-H", cmaps),
                regroup: true,
                to_unicode: load_cmap(b"Adobe-GB1-UCS2", cmaps)
                    .map_or(ToUnicode::None, ToUnicode::Map),
                collection: Collection::None,
                simple: None,
            };
        }
        Self {
            encoding: None,
            regroup: false,
            to_unicode,
            collection: Collection::None,
            simple: Some(simple_table(dict, program)),
        }
    }

    fn composite(
        dict: &Dict<'_>,
        to_unicode: ToUnicode,
        embedded: bool,
        cmaps: &mut CMaps,
    ) -> Self {
        let encoding = match dict.get::<Object<'_>>(ENCODING) {
            Some(Object::Name(n)) => load_cmap(n.as_ref(), cmaps),
            Some(Object::Stream(s)) => parse_cmap(&s).map(Rc::new),
            _ => None,
        };
        let descendant = dict
            .get::<Array<'_>>(DESCENDANT_FONTS)
            .and_then(|a| a.iter::<Dict<'_>>().next());
        let collection = match &to_unicode {
            ToUnicode::None => {
                let family = descendant
                    .as_ref()
                    .and_then(|d| d.get::<Dict<'_>>(CIDSYSTEMINFO))
                    .and_then(|info| {
                        let registry = info.get::<PdfString<'_>>(REGISTRY)?;
                        let ordering = info.get::<PdfString<'_>>(ORDERING)?;
                        Some(CidFamily::from_registry_ordering(
                            registry.as_bytes(),
                            ordering.as_bytes(),
                        ))
                    });
                let ucs2 = family.as_ref().and_then(CidFamily::ucs2_cmap);
                let identity = encoding.as_ref().is_some_and(|e| {
                    e.metadata()
                        .name
                        .as_ref()
                        .is_some_and(|n| n.windows(9).any(|w| w == b"Identity-"))
                });
                match ucs2.and_then(|name| load_cmap(name.to_bytes(), cmaps)) {
                    Some(cmap) => Collection::Ucs2(cmap),
                    None if identity && !embedded => Collection::TrueType,
                    None => Collection::None,
                }
            }
            _ => Collection::None,
        };
        Self {
            encoding,
            regroup: false,
            to_unicode,
            collection,
            simple: None,
        }
    }

    /// The advance in ems MuPDF gives every code, where it ignores the
    /// font's widths: its cp936 reading has the collection's default 1000.
    pub(super) fn advance(&self) -> Option<f64> {
        self.regroup.then_some(1.0)
    }

    /// Whether the font's encoding writes vertically, as hayro reads it:
    /// glyphs are then displaced from the pen.
    pub(super) fn vertical(&self) -> bool {
        !self.regroup
            && (self.encoding.as_ref())
                .is_some_and(|e| e.metadata().writing_mode == Some(WritingMode::Vertical))
    }

    /// Whether hayro draws one glyph per byte of shown text (a simple font).
    pub(super) fn one_byte_glyphs(&self) -> bool {
        self.encoding.is_none() || self.regroup
    }

    /// Splits shown bytes into codes as hayro does for a composite font (the
    /// first prefix of one to four bytes the encoding maps), else one per byte.
    pub(super) fn codes(&self, bytes: &[u8], out: &mut Vec<Code>) {
        let Some(cmap) = &self.encoding else {
            out.extend(bytes.iter().map(|&b| Code {
                code: u32::from(b),
                cid: u32::from(b),
                bytes: 1,
            }));
            return;
        };
        let mut at = 0;
        while at < bytes.len() {
            let mut read = (0, 1);
            let mut code = 0u32;
            for n in 0..4.min(bytes.len() - at) {
                code = (code << 8) | u32::from(bytes[at + n]);
                if cmap.lookup_cid_code(code, n as u8 + 1).is_some() {
                    read = (code, n + 1);
                    break;
                }
            }
            let (code, len) = read;
            out.push(Code {
                code,
                cid: cid_of(cmap, code).unwrap_or(0),
                bytes: len as u8,
            });
            at += len;
        }
    }

    /// The text of one code, or `None` where hayro's own reading stands.
    pub(super) fn text(&self, code: Code) -> Option<Vec<char>> {
        let from_map = match &self.to_unicode {
            ToUnicode::Map(map) => {
                let key = if self.regroup { code.cid } else { code.code };
                // A ToUnicode CID mapping gives the CID as the character.
                mapped(map.lookup_bf_string(key)).or_else(|| {
                    cid_of(map, key)
                        .and_then(char::from_u32)
                        .and_then(mapped_char)
                        .map(|c| vec![c])
                })
            }
            ToUnicode::Named(cmap) => cid_of(cmap, code.code)
                .and_then(char::from_u32)
                .and_then(mapped_char)
                .map(|c| vec![c]),
            ToUnicode::Broken | ToUnicode::None => None,
        };
        if from_map.is_some() {
            return from_map;
        }
        if let Some(table) = &self.simple {
            let slot = usize::try_from(code.code).ok().and_then(|c| table.get(c));
            return match slot {
                Some(Some(c)) => Some(vec![*c]),
                Some(None) => None,
                None => Some(vec![FFFD]),
            };
        }
        let c = match (&self.to_unicode, &self.collection) {
            (ToUnicode::None, Collection::Ucs2(cmap)) => {
                return Some(mapped(cmap.lookup_bf_string(code.cid)).unwrap_or(vec![FFFD]));
            }
            (ToUnicode::None, Collection::TrueType) => usize::try_from(code.cid)
                .ok()
                .and_then(|cid| data::TRUETYPE_UCS2.get(cid))
                .and_then(|&v| char::from_u32(u32::from(v)))
                .and_then(mapped_char),
            _ => None,
        };
        Some(vec![c.unwrap_or(FFFD)])
    }
}

/// MuPDF's S22PDF workaround: a Chinese font declared WinAnsi with Flags 4
/// whose BaseFont is one of five GBK-encoded face names.
fn cp936_font(dict: &Dict<'_>) -> bool {
    const NAMES: [&[u8]; 5] = [
        b"\xCB\xCE\xCC\xE5",
        b"\xBA\xDA\xCC\xE5",
        b"\xBF\xAC\xCC\xE5_GB2312",
        b"\xB7\xC2\xCB\xCE_GB2312",
        b"\xC1\xA5\xCA\xE9",
    ];
    let flags = dict
        .get::<Dict<'_>>(FONT_DESC)
        .and_then(|d| d.get::<i64>(FLAGS));
    let winansi = dict
        .get::<Name<'_>>(ENCODING)
        .is_some_and(|n| n.as_ref() == b"WinAnsiEncoding");
    let base = dict.get::<Name<'_>>(BASE_FONT);
    flags == Some(4) && winansi && base.is_some_and(|b| NAMES.contains(&b.as_ref()))
}

/// The glyph name of each code from an encoding object: a named base, then
/// /Differences. `default` is the base when the font has no encoding, and
/// for a simple font (not Type3) when its encoding dictionary names none.
pub(super) fn encoding_names(
    encoding: Option<Object<'_>>,
    default: Option<NamedEncoding>,
    type3: bool,
) -> Vec<Option<Vec<u8>>> {
    let mut names: Vec<Option<Vec<u8>>> = vec![None; 256];
    let (base, differences) = match encoding {
        Some(Object::Name(n)) => (NamedEncoding::from_pdf_name(n.as_ref()), None),
        Some(Object::Dict(d)) => (
            match d.get::<Name<'_>>(BASE_ENCODING) {
                Some(n) => NamedEncoding::from_pdf_name(n.as_ref()),
                None if type3 => None,
                None => default,
            },
            d.get::<Array<'_>>(DIFFERENCES),
        ),
        Some(_) => (None, None),
        None => (default, None),
    };
    if let Some(base) = base {
        let table = SimpleEncoding::from_named(base);
        for (code, slot) in names.iter_mut().enumerate() {
            *slot = table.glyph_name(code as u8).map(<[u8]>::to_vec);
        }
    }
    let mut code: i64 = 0;
    for item in differences.iter().flat_map(|a| a.iter::<Object<'_>>()) {
        match item {
            Object::Number(n) => code = n.as_i64(),
            Object::Name(n) => {
                if let Some(slot) = usize::try_from(code).ok().and_then(|c| names.get_mut(c)) {
                    *slot = Some(n.as_ref().to_vec());
                }
                code = code.saturating_add(1);
            }
            _ => {}
        }
    }
    names
}

/// MuPDF's Type3 text: the encoding's glyph names (Standard when the font
/// names no encoding), and a printable ASCII code without a known name as itself.
fn type3_table(dict: &Dict<'_>) -> Box<[Option<char>; 256]> {
    let names = encoding_names(
        dict.get::<Object<'_>>(ENCODING),
        Some(NamedEncoding::Standard),
        true,
    );
    Box::new(std::array::from_fn(|code| {
        let c = names[code].as_deref().map_or(FFFD, glyph_name);
        Some(if c == FFFD && (0x20..0x7f).contains(&code) {
            char::from(code as u8)
        } else {
            c
        })
    }))
}

/// The built-in face MuPDF draws a font with when its program is not embedded.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Builtin {
    Standard,
    Symbol,
    Dingbats,
}

fn builtin_face(base: &[u8]) -> Builtin {
    match base {
        b"Symbol"
        | b"Symbol,Italic"
        | b"Symbol,Bold"
        | b"Symbol,BoldItalic"
        | b"SymbolMT"
        | b"SymbolMT,Italic"
        | b"SymbolMT,Bold"
        | b"SymbolMT,BoldItalic" => Builtin::Symbol,
        b"ZapfDingbats" => Builtin::Dingbats,
        _ => Builtin::Standard,
    }
}

/// MuPDF's simple-font text (`pdf_load_simple_font`): the encoding's glyph
/// names, else the names of the font's built-in encoding.
fn simple_table(dict: &Dict<'_>, data: Option<&[u8]>) -> Box<[Option<char>; 256]> {
    let base = dict
        .get::<Name<'_>>(BASE_FONT)
        .map(|n| n.as_ref().to_vec())
        .unwrap_or_default();
    let descriptor = dict.get::<Dict<'_>>(FONT_DESC);
    let builtin = builtin_face(&base);
    let flags = match &descriptor {
        Some(d) => d.get::<i64>(FLAGS).unwrap_or(0),
        None if builtin != Builtin::Standard => 4,
        None => 0,
    };
    let symbolic = flags & 4 != 0 && flags & 32 == 0;
    let truetype = data.and_then(Sfnt::read);
    let embedded = data.is_some();
    let default = (!embedded && !symbolic).then_some(NamedEncoding::Standard);
    let names = encoding_names(dict.get::<Object<'_>>(ENCODING), default, false);
    let standard = SimpleEncoding::from_named(NamedEncoding::Standard);
    let winansi = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    // The glyph of each code, else of 0xF000 plus the code (symbol fonts).
    let mut gids = [0u16; 256];
    if let Some(font) = &truetype
        && let Some(cmap) = font.charmap(symbolic)
    {
        let low = font.glyphs(cmap, 0..=255);
        let high = font.glyphs(cmap, 0xf000..=0xf0ff);
        for (gid, (low, high)) in gids.iter_mut().zip(low.into_iter().zip(high)) {
            *gid = if low == 0 { high } else { low };
        }
    }
    Box::new(std::array::from_fn(|code| {
        if let Some(name) = &names[code] {
            return Some(glyph_name(name));
        }
        let byte = code as u8;
        if !embedded {
            let c = match builtin {
                Builtin::Symbol => char::from_u32(u32::from(data::SYMBOL[code])),
                Builtin::Dingbats => char::from_u32(u32::from(data::DINGBATS[code])),
                Builtin::Standard => standard.glyph_name(byte).map(glyph_name),
            };
            return Some(c.unwrap_or(FFFD));
        }
        let Some(font) = &truetype else {
            return None;
        };
        let gid = gids[code];
        if gid == 0 {
            return Some(FFFD);
        }
        Some(match font.names {
            PostNames::None => winansi.glyph_name(byte).map_or(FFFD, glyph_name),
            _ => font.glyph_name(gid).map_or(FFFD, |n| glyph_name(&n)),
        })
    }))
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        d.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        d.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

enum PostNames {
    /// No `post` table, format 3 or another without names: FreeType reports no glyph names.
    None,
    Format1,
    /// Format 2: per-glyph indexes (at `at`) into the Macintosh order, then
    /// names past it, at the offsets of `strings`.
    Format2 {
        count: usize,
        at: usize,
        strings: Vec<usize>,
    },
}

/// A TrueType program: its cmap subtables and its glyph names.
struct Sfnt<'a> {
    data: &'a [u8],
    /// (platform, encoding, subtable offset) in record order.
    cmaps: Vec<(u16, u16, usize)>,
    names: PostNames,
}

impl<'a> Sfnt<'a> {
    /// Reads a TrueType (`glyf`) program; OpenType CFF programs are not read.
    fn read(data: &'a [u8]) -> Option<Self> {
        if !matches!(u32_at(data, 0)?, 0x0001_0000 | 0x7472_7565) {
            return None;
        }
        let count = usize::from(u16_at(data, 4)?);
        let table = |tag: &[u8; 4]| {
            (0..count).find_map(|i| {
                let rec = 12 + 16 * i;
                (data.get(rec..rec + 4)? == tag)
                    .then(|| usize::try_from(u32_at(data, rec + 8)?).ok())
                    .flatten()
            })
        };
        table(b"glyf")?;
        let mut cmaps = Vec::new();
        if let Some(cmap) = table(b"cmap") {
            let records = usize::from(u16_at(data, cmap + 2).unwrap_or(0));
            for i in 0..records {
                let rec = cmap + 4 + 8 * i;
                let (Some(platform), Some(encoding), Some(offset)) = (
                    u16_at(data, rec),
                    u16_at(data, rec + 2),
                    u32_at(data, rec + 4),
                ) else {
                    break;
                };
                if let Some(at) = usize::try_from(offset)
                    .ok()
                    .and_then(|o| cmap.checked_add(o))
                {
                    cmaps.push((platform, encoding, at));
                }
            }
        }
        let names = match table(b"post") {
            Some(post) => match u32_at(data, post) {
                Some(0x0001_0000) => PostNames::Format1,
                Some(0x0002_0000) => match u16_at(data, post + 32) {
                    Some(n) => {
                        let count = usize::from(n);
                        let mut strings = Vec::new();
                        let mut pos = post + 34 + 2 * count;
                        while strings.len() < usize::from(u16::MAX)
                            && let Some(&len) = data.get(pos)
                        {
                            strings.push(pos);
                            pos += 1 + usize::from(len);
                        }
                        PostNames::Format2 {
                            count,
                            at: post + 34,
                            strings,
                        }
                    }
                    None => PostNames::None,
                },
                _ => PostNames::None,
            },
            None => PostNames::None,
        };
        Some(Self { data, cmaps, names })
    }

    fn format(&self, at: usize) -> Option<u16> {
        u16_at(self.data, at).filter(|f| matches!(f, 0 | 2 | 4 | 6 | 8 | 10 | 12 | 13 | 14))
    }

    /// MuPDF's `select_truetype_cmap`: (3,0) first for a symbolic font, then
    /// (3,1), (1,0) and the first subtable of a known format.
    fn charmap(&self, symbolic: bool) -> Option<usize> {
        let find = |platform: u16, encoding: u16, valid: bool| {
            self.cmaps
                .iter()
                .find(|(p, e, at)| {
                    *p == platform && *e == encoding && (!valid || self.format(*at).is_some())
                })
                .map(|c| c.2)
        };
        if symbolic && let Some(at) = find(3, 0, false) {
            return Some(at);
        }
        find(3, 1, true).or_else(|| find(1, 0, true)).or_else(|| {
            self.cmaps
                .first()
                .map(|c| c.2)
                .filter(|at| self.format(*at).is_some())
        })
    }

    /// The glyphs a cmap subtable gives 256 consecutive character codes
    /// (0 for none): the first segment ending at or after a code decides it
    /// (format 4, one pass over the segments), the group holding it
    /// (formats 12 and 13).
    fn glyphs(&self, at: usize, codes: std::ops::RangeInclusive<u32>) -> [u16; 256] {
        let d = self.data;
        let first = *codes.start();
        let mut out = [0u16; 256];
        let slot = |code: u32| usize::try_from(code - first).ok();
        match u16_at(d, at) {
            Some(0) => {
                for code in codes.filter(|c| *c < 256) {
                    if let (Some(i), Some(&g)) = (slot(code), d.get(at + 6 + code as usize)) {
                        out[i] = u16::from(g);
                    }
                }
            }
            Some(4) => {
                let Some(segs) = u16_at(d, at + 6).map(|n| usize::from(n / 2)) else {
                    return out;
                };
                let ends = at + 14;
                let starts = ends + 2 * segs + 2;
                let deltas = starts + 2 * segs;
                let ranges = deltas + 2 * segs;
                // Codes up to `done` are decided by an earlier segment.
                let mut done: Option<u32> = None;
                for i in 0..segs {
                    let Some(end) = u16_at(d, ends + 2 * i).map(u32::from) else {
                        break;
                    };
                    let from = done.map_or(first, |d| d.saturating_add(1)).max(first);
                    if done.is_some_and(|d| end <= d) {
                        continue;
                    }
                    done = Some(end);
                    let (Some(start), Some(delta), Some(range)) = (
                        u16_at(d, starts + 2 * i),
                        u16_at(d, deltas + 2 * i),
                        u16_at(d, ranges + 2 * i).map(usize::from),
                    ) else {
                        continue;
                    };
                    for code in from.max(u32::from(start))..=end.min(*codes.end()) {
                        let Some(i_out) = slot(code) else { continue };
                        let code = code as u16;
                        out[i_out] = if range == 0 {
                            code.wrapping_add(delta)
                        } else {
                            let at = ranges + 2 * i + range + 2 * usize::from(code - start);
                            match u16_at(d, at) {
                                Some(0) | None => 0,
                                Some(glyph) => glyph.wrapping_add(delta),
                            }
                        };
                    }
                    if end >= *codes.end() {
                        break;
                    }
                }
            }
            Some(6) => {
                let (Some(start), Some(count)) = (u16_at(d, at + 6), u16_at(d, at + 8)) else {
                    return out;
                };
                for code in codes {
                    let i = code
                        .checked_sub(u32::from(start))
                        .filter(|i| *i < u32::from(count));
                    if let (Some(i_out), Some(glyph)) = (
                        slot(code),
                        i.and_then(|i| u16_at(d, at + 10 + 2 * i as usize)),
                    ) {
                        out[i_out] = glyph;
                    }
                }
            }
            // Groups are sorted by code, as FreeType requires (it rejects
            // the subtable otherwise): each code is looked up by bisection.
            Some(format @ (12 | 13)) => {
                let groups = u32_at(d, at + 12).map_or(0, |n| n as usize);
                let room = d.len().saturating_sub(at + 16) / 12;
                let groups = groups.min(room);
                let group = |i: usize| {
                    let g = at + 16 + 12 * i;
                    (u32_at(d, g), u32_at(d, g + 4), u32_at(d, g + 8))
                };
                for code in codes {
                    let (mut lo, mut hi) = (0, groups);
                    while lo < hi {
                        let mid = lo + (hi - lo) / 2;
                        if group(mid).0.is_some_and(|start| start <= code) {
                            lo = mid + 1;
                        } else {
                            hi = mid;
                        }
                    }
                    let Some((Some(start), Some(end), Some(glyph))) = lo.checked_sub(1).map(group)
                    else {
                        continue;
                    };
                    if code > end {
                        continue;
                    }
                    let glyph = if format == 12 {
                        glyph.checked_add(code - start)
                    } else {
                        Some(glyph)
                    };
                    if let Some(i_out) = slot(code) {
                        out[i_out] = glyph.and_then(|g| u16::try_from(g).ok()).unwrap_or(0);
                    }
                }
            }
            _ => {}
        }
        out
    }

    /// The `post` name of a glyph, as FreeType gives it.
    fn glyph_name(&self, gid: u16) -> Option<Vec<u8>> {
        let gid = usize::from(gid);
        let (index, strings) = match &self.names {
            PostNames::None => return None,
            PostNames::Format1 => (gid, &[][..]),
            PostNames::Format2 { count, at, strings } => {
                if gid >= *count {
                    return None;
                }
                (usize::from(u16_at(self.data, at + 2 * gid)?), &strings[..])
            }
        };
        if let Some(name) = data::MAC_GLYPHS.get(index) {
            return Some(name.to_vec());
        }
        let pos = *strings.get(index - data::MAC_GLYPHS.len())?;
        let len = usize::from(*self.data.get(pos)?);
        Some(self.data.get(pos + 1..pos + 1 + len)?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `glyf` program whose only cmap subtable is `subtable`.
    fn program(subtable: &[u8]) -> Vec<u8> {
        let mut cmap = Vec::new();
        for v in [0u16, 1, 3, 1] {
            cmap.extend(v.to_be_bytes());
        }
        cmap.extend(12u32.to_be_bytes());
        cmap.extend(subtable);
        let tables: [(&[u8; 4], Vec<u8>); 2] = [(b"cmap", cmap), (b"glyf", vec![0; 4])];
        let mut out = 0x0001_0000u32.to_be_bytes().to_vec();
        for v in [2u16, 0, 0, 0] {
            out.extend(v.to_be_bytes());
        }
        let mut offset = 12 + 16 * tables.len();
        for (tag, data) in &tables {
            out.extend(*tag);
            for v in [0, offset as u32, data.len() as u32] {
                out.extend(v.to_be_bytes());
            }
            offset += data.len();
        }
        for (_, data) in &tables {
            out.extend(data);
        }
        out
    }

    /// The lookup the one pass replaces: the first segment ending at or
    /// after the code decides it.
    fn format4_by_code(segments: &[(u16, u16, u16)], code: u16) -> u16 {
        for &(start, end, delta) in segments {
            if end < code {
                continue;
            }
            return if start > code {
                0
            } else {
                code.wrapping_add(delta)
            };
        }
        0
    }

    #[test]
    fn format_4_segments_decide_codes_as_a_lookup_by_code_would() {
        // Ends out of order, overlaps, and a segment ending below the codes.
        let segments = [
            (0x10, 0x20, 100),
            (0x05, 0x18, 200),
            (0x00, 0x08, 300),
            (0x41, 0x60, 1),
            (0x30, 0x50, 2),
            (0xf020, 0xf030, 7),
            (0xffff, 0xffff, 1),
        ];
        let n = segments.len() as u16;
        let mut sub = Vec::new();
        for v in [4, 0, 0, 2 * n, 0, 0, 0] {
            sub.extend(v.to_be_bytes());
        }
        let column = |sub: &mut Vec<u8>, f: fn(&(u16, u16, u16)) -> u16| {
            for s in &segments {
                sub.extend(f(s).to_be_bytes());
            }
        };
        column(&mut sub, |s| s.1);
        sub.extend([0, 0]);
        column(&mut sub, |s| s.0);
        column(&mut sub, |s| s.2);
        column(&mut sub, |_| 0);
        let data = program(&sub);
        let font = Sfnt::read(&data).unwrap();
        let at = font.charmap(false).unwrap();
        for range in [0..=255u32, 0xf000..=0xf0ff] {
            let first = *range.start();
            let glyphs = font.glyphs(at, range.clone());
            for code in range {
                let want = format4_by_code(&segments, code as u16);
                assert_eq!(glyphs[(code - first) as usize], want, "{code:#x}");
            }
        }
    }

    #[test]
    fn format_12_groups_are_found_by_bisection() {
        let groups = [
            (0x20u32, 0x22u32, 5u32),
            (0x41, 0x5a, 40),
            (0xf041, 0xf041, 9),
        ];
        let mut sub = Vec::new();
        for v in [12u16, 0] {
            sub.extend(v.to_be_bytes());
        }
        for v in [16 + 12 * groups.len() as u32, 0, groups.len() as u32] {
            sub.extend(v.to_be_bytes());
        }
        for (start, end, glyph) in groups {
            for v in [start, end, glyph] {
                sub.extend(v.to_be_bytes());
            }
        }
        let data = program(&sub);
        let font = Sfnt::read(&data).unwrap();
        let at = font.charmap(false).unwrap();
        let low = font.glyphs(at, 0..=255);
        assert_eq!(&low[0x1f..0x24], [0, 5, 6, 7, 0]);
        assert_eq!((low[0x41], low[0x5a], low[0x5b]), (40, 65, 0));
        let high = font.glyphs(at, 0xf000..=0xf0ff);
        assert_eq!((high[0x40], high[0x41], high[0x42]), (0, 9, 0));
        // A table claiming more groups than its bytes hold reads what is there.
        sub[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        let data = program(&sub);
        let font = Sfnt::read(&data).unwrap();
        assert_eq!(font.glyphs(at, 0..=255)[0x41], 40);
    }
}
