// SPDX-License-Identifier: MIT
//! The signature dictionary as signed: its `/M` (the signer's claimed time)
//! read from the bytes the signature covers, around the `/Contents` gap.
//!
//! A PAdES signature carries its time only in `/M`. A later incremental
//! update can redefine the signature dictionary (same `/ByteRange` and
//! `/Contents`, another `/M`), and a reader that takes the newest version
//! then reports a time nobody signed. The dictionary that holds the gap is,
//! byte for byte, the one in the signed revision, so its `/M` is the signed
//! one: it is read here with a small PDF tokenizer, from signed bytes only.
//!
//! The answer has three states ([`SignedM`]): the dictionary could not be
//! read (then nothing is claimed about the time, either way), it has no
//! `/M`, or its `/M` text.

use crate::source::Source;

/// How far before the gap the dictionary may start, and after it end.
const WINDOW: u64 = 64 * 1024;

/// What the signed signature dictionary says about `/M`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignedM {
    /// The dictionary holding the gap could not be read with confidence (no
    /// object header within reach, an unexpected structure, an indirect or
    /// repeated `/M`).
    Unreadable,
    /// The dictionary was read and has no `/M`.
    Absent,
    /// The signed `/M`, as text.
    Text(String),
}

impl SignedM {
    /// The text, when there is one.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(t) => Some(t),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    DictOpen,
    DictClose,
    ArrayOpen,
    ArrayClose,
    Name(Vec<u8>),
    Str(Vec<u8>),
    /// A number, `R`, `true`, `null`, `endobj` or another bare word.
    Word(Vec<u8>),
}

fn is_white(b: u8) -> bool {
    matches!(b, b' ' | b'\n' | b'\r' | b'\t' | b'\x0c' | 0)
}

fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn hexval(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// A streaming tokenizer: tokens are read one at a time, so reading stops at
/// the end of the dictionary and never looks at what follows `endobj`.
struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lexer<'_> {
    /// The next token: `Some(Ok)`, `Some(Err)` when the bytes are not a
    /// token (or run off the end inside one), `None` at the end.
    fn next_tok(&mut self) -> Option<Result<Tok, ()>> {
        let s = self.s;
        loop {
            let b = *s.get(self.i)?;
            if is_white(b) {
                self.i += 1;
            } else if b == b'%' {
                while self.i < s.len() && s[self.i] != b'\n' && s[self.i] != b'\r' {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
        let rest = &s[self.i..];
        if rest.starts_with(b"<<") {
            self.i += 2;
            return Some(Ok(Tok::DictOpen));
        }
        if rest.starts_with(b">>") {
            self.i += 2;
            return Some(Ok(Tok::DictClose));
        }
        let b = rest[0];
        self.i += 1;
        Some(match b {
            b'[' => Ok(Tok::ArrayOpen),
            b']' => Ok(Tok::ArrayClose),
            b'/' => Ok(Tok::Name(self.name())),
            b'<' => self.hex(),
            b'(' => self.literal(),
            _ if is_delim(b) => Err(()),
            _ => {
                let start = self.i - 1;
                while self.i < s.len() && !is_white(s[self.i]) && !is_delim(s[self.i]) {
                    self.i += 1;
                }
                Ok(Tok::Word(s[start..self.i].to_vec()))
            }
        })
    }

    /// A name after its `/`, with `#xx` escapes decoded.
    fn name(&mut self) -> Vec<u8> {
        let s = self.s;
        let mut out = Vec::new();
        while self.i < s.len() && !is_white(s[self.i]) && !is_delim(s[self.i]) {
            let b = s[self.i];
            if b == b'#'
                && let (Some(h), Some(l)) = (
                    s.get(self.i + 1).copied().and_then(hexval),
                    s.get(self.i + 2).copied().and_then(hexval),
                )
            {
                out.push(h << 4 | l);
                self.i += 3;
                continue;
            }
            out.push(b);
            self.i += 1;
        }
        out
    }

    fn hex(&mut self) -> Result<Tok, ()> {
        let mut digits = Vec::new();
        loop {
            let c = *self.s.get(self.i).ok_or(())?;
            self.i += 1;
            if c == b'>' {
                break;
            }
            if !is_white(c) {
                digits.push(hexval(c).ok_or(())?);
            }
        }
        if digits.len() % 2 == 1 {
            digits.push(0);
        }
        Ok(Tok::Str(
            digits.chunks(2).map(|p| p[0] << 4 | p[1]).collect(),
        ))
    }

    fn literal(&mut self) -> Result<Tok, ()> {
        let s = self.s;
        let mut depth = 1;
        let mut text = Vec::new();
        loop {
            let c = *s.get(self.i).ok_or(())?;
            self.i += 1;
            match c {
                b'\\' => {
                    let e = *s.get(self.i).ok_or(())?;
                    self.i += 1;
                    match e {
                        b'n' => text.push(b'\n'),
                        b'r' => text.push(b'\r'),
                        b't' => text.push(b'\t'),
                        b'b' => text.push(8),
                        b'f' => text.push(12),
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match s.get(self.i) {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.i += 1;
                                    }
                                    _ => break,
                                }
                            }
                            text.push((v & 0xff) as u8);
                        }
                        b'\r' => {
                            if s.get(self.i) == Some(&b'\n') {
                                self.i += 1;
                            }
                        }
                        b'\n' => {}
                        other => text.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    text.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(Tok::Str(text));
                    }
                    text.push(c);
                }
                _ => text.push(c),
            }
        }
    }
}

/// Walks the signature dictionary's tokens, pairing keys and values at its
/// own level (nested dictionaries and arrays, and `n g R` references, are one
/// value each).
#[derive(Default)]
struct Walker {
    /// Nesting below the signature dictionary (0 = its own level).
    depth: usize,
    started: bool,
    closed: bool,
    key: Option<Vec<u8>>,
    /// Words seen in the current value at level 0 (for `n g R`).
    words: usize,
    m: Option<Vec<u8>>,
    /// `/M` was repeated, or was not a string (an indirect reference).
    m_unusable: bool,
}

impl Walker {
    /// Feeds one token; `false` when the structure is not a dictionary.
    fn feed(&mut self, t: &Tok) -> bool {
        if self.closed {
            return true;
        }
        if !self.started {
            self.started = true;
            return *t == Tok::DictOpen;
        }
        if self.depth > 0 {
            match t {
                Tok::DictOpen | Tok::ArrayOpen => self.depth += 1,
                Tok::DictClose | Tok::ArrayClose => {
                    self.depth -= 1;
                    if self.depth == 0 {
                        self.key = None;
                        self.words = 0;
                    }
                }
                _ => {}
            }
            return true;
        }
        let is_m = self.key.as_deref() == Some(b"M".as_slice());
        match (t, &self.key) {
            (Tok::DictClose, _) => self.closed = true,
            (Tok::Name(n), None) => {
                self.key = Some(n.clone());
                self.words = 0;
            }
            (_, None) => {
                // A bare word after a value: the `g R` of a reference.
                if !matches!(t, Tok::Word(_)) || self.words == 0 || self.words > 2 {
                    return false;
                }
                self.words += 1;
            }
            (Tok::DictOpen | Tok::ArrayOpen, Some(_)) => {
                self.m_unusable |= is_m;
                self.depth = 1;
            }
            (Tok::ArrayClose, Some(_)) => return false,
            (Tok::Str(s), Some(_)) => {
                if is_m {
                    if self.m.is_some() {
                        self.m_unusable = true;
                    }
                    self.m = Some(s.clone());
                }
                self.key = None;
                self.words = 0;
            }
            (Tok::Word(_), Some(_)) => {
                self.m_unusable |= is_m;
                self.key = None;
                self.words = 1;
            }
            (Tok::Name(_), Some(_)) => {
                self.m_unusable |= is_m;
                self.key = None;
                self.words = 0;
            }
        }
        true
    }
}

/// A PDF text string as Rust text: UTF-16BE or UTF-16LE with their byte
/// order marks, UTF-8 with its BOM, else PDFDocEncoding (read as Latin-1,
/// which agrees for the characters of a date).
fn text(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], le: bool| -> String {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                if le {
                    u16::from_le_bytes(*c)
                } else {
                    u16::from_be_bytes(*c)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        utf16(rest, false)
    } else if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        utf16(rest, true)
    } else if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        String::from_utf8_lossy(rest).into_owned()
    } else {
        bytes.iter().map(|&b| char::from(b)).collect()
    }
}

/// Where the dictionary holding the gap starts: just after the `N G obj`
/// that opens the last object before the gap. A candidate in a comment (a
/// `%` earlier on its line) is skipped.
fn object_start(before: &[u8]) -> Option<usize> {
    // For each position, whether a `%` comes earlier on its line (one pass,
    // so a long line of candidates costs time in proportion to its length).
    let mut commented = Vec::with_capacity(before.len());
    let mut seen = false;
    for &c in before {
        commented.push(seen);
        if c == b'\n' || c == b'\r' {
            seen = false;
        } else if c == b'%' {
            seen = true;
        }
    }
    let mut at = before.len();
    while at >= 3 {
        let pos = before[..at].windows(3).rposition(|w| w == b"obj")?;
        at = pos;
        let after_ok = before
            .get(pos + 3)
            .is_none_or(|&c| is_white(c) || c == b'<' || c == b'%');
        let head = &before[..pos];
        let mut j = head.len();
        let mut fields = 0;
        // Expect: digits, white, digits, white, then "obj".
        while fields < 2 {
            let white_end = j;
            while j > 0 && is_white(head[j - 1]) {
                j -= 1;
            }
            if j == white_end {
                break;
            }
            let digits_end = j;
            while j > 0 && head[j - 1].is_ascii_digit() {
                j -= 1;
            }
            if j == digits_end {
                break;
            }
            fields += 1;
        }
        if !after_ok || fields != 2 || (j > 0 && !is_white(head[j - 1])) {
            continue;
        }
        if commented[j] {
            continue;
        }
        return Some(pos + 3);
    }
    None
}

/// The signed `/M` of the signature whose `/Contents` gap is `[b, c)` in
/// `src`, the signature covering bytes up to `covered`: read from the
/// dictionary that holds the gap, and only from signed bytes.
pub fn signed_m(src: &dyn Source, b: u64, c: u64, covered: u64) -> SignedM {
    read(src, b, c, covered).unwrap_or(SignedM::Unreadable)
}

fn read(src: &dyn Source, b: u64, c: u64, covered: u64) -> Option<SignedM> {
    let from = b.saturating_sub(WINDOW);
    let before = src.read_at(from, usize::try_from(b - from).ok()?)?;
    let after_len = covered.checked_sub(c)?.min(WINDOW);
    let after = src.read_at(c, usize::try_from(after_len).ok()?)?;
    let start = object_start(&before)?;
    let mut walker = Walker::default();
    let mut head = Lexer {
        s: &before[start..],
        i: 0,
    };
    while let Some(t) = head.next_tok() {
        let t = t.ok()?;
        if !walker.feed(&t) || walker.closed {
            return None;
        }
    }
    // The gap must be the value of /Contents at the dictionary's own level.
    if walker.depth != 0 || walker.key.as_deref() != Some(b"Contents".as_slice()) {
        return None;
    }
    walker.key = None;
    // The tail, token by token, to the dictionary's `>>` and the `endobj` after it.
    let mut tail = Lexer { s: &after, i: 0 };
    while !walker.closed {
        let t = tail.next_tok()?.ok()?;
        if !walker.feed(&t) {
            return None;
        }
    }
    if tail.next_tok()?.ok()? != Tok::Word(b"endobj".to_vec()) {
        return None;
    }
    if walker.m_unusable {
        return None;
    }
    Some(
        walker
            .m
            .map_or(SignedM::Absent, |m| SignedM::Text(text(&m))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gap(pdf: &[u8]) -> (u64, u64) {
        let b = pdf.windows(10).position(|w| w == b"/Contents ").unwrap() + 10;
        let c = b + pdf[b..].iter().position(|&x| x == b'>').unwrap() + 1;
        (b as u64, c as u64)
    }

    fn m(pdf: &[u8]) -> SignedM {
        let (b, c) = gap(pdf);
        let src: &[u8] = pdf;
        signed_m(&src, b, c, pdf.len() as u64)
    }

    fn text_m(t: &str) -> SignedM {
        SignedM::Text(t.to_string())
    }

    #[test]
    fn reads_the_m_of_the_dictionary_holding_the_gap() {
        let pdf: &[u8] = b"%PDF-1.7\n9 0 obj\n<</V 10 0 R/M(D:19990101000000Z)>>\nendobj\n10 0 obj\n<<\n/Prop_Build << /App << /Name /X >> >> /Reference [ << /P 2 >> ] /Ref 3 0 R\n/Contents <3082ABCD>\n/ByteRange [0 1 2 3] /M (D\\07220260916154752\\05504\\04700\\047) /Name (a\\)b) >>\nendobj\n11 0 obj << /M (D:20990101000000Z) >> endobj\n";
        assert_eq!(m(pdf), text_m("D:20260916154752-04'00'"));
    }

    #[test]
    fn three_states() {
        assert_eq!(
            m(b"1 0 obj\n<< /Contents <3082> /Name (x) >>\nendobj\n"),
            SignedM::Absent
        );
        assert_eq!(
            m(b"1 0 obj\n<< /Contents <3082> /M (D:20260101000000Z)"),
            SignedM::Unreadable,
            "no closing >>"
        );
        let pdf: &[u8] = b"1 0 obj\n<< /Other <00> /Contents <3082> >>\nendobj\n";
        let b = pdf.windows(9).position(|w| w == b"/Other <0").unwrap() as u64 + 7;
        let src: &[u8] = pdf;
        assert_eq!(
            signed_m(&src, b, b + 4, pdf.len() as u64),
            SignedM::Unreadable
        );
    }

    #[test]
    fn review_p2c_hostile_cases() {
        let ok = text_m("D:20200101000000Z");
        assert_eq!(
            m(b"1 0 obj\n<< /M (D:20200101000000Z) /Contents <3082> /ByteRange [0 1 2 3] >>\nendobj\n"),
            ok
        );
        assert_eq!(
            m(b"1 0 obj\n<< /M (D:20200101000000Z) % 2 0 obj<</M(D:19990101000000Z)\n/Contents <3082> >>\nendobj\n"),
            ok,
            "a header in a comment is not a header"
        );
        assert_eq!(
            m(b"1 0 obj\n<< /M (D:20200101000000Z) /M (D:19990101000000Z) /Contents <3082> >>\nendobj\n"),
            SignedM::Unreadable,
            "a repeated /M gives no date"
        );
        assert_eq!(
            m(b"1 0 obj\n<< /M (D:20200101000000Z) /Contents <3082> /M (D:19990101000000Z) >>\nendobj\n"),
            SignedM::Unreadable
        );
        assert_eq!(
            m(b"1 0 obj\n<< /P << /M (D:19990101000000Z) >> /Contents <3082> /R [ /M (D:1998) ] >>\nendobj\n"),
            SignedM::Absent,
            "nested /M is not the dictionary's"
        );
        assert_eq!(
            m(b"1 0 obj\n<< /#4D (D:19990101000000Z) /Contents <3082> >>\nendobj\n"),
            text_m("D:19990101000000Z")
        );
        assert_eq!(
            m(b"1 0 obj\n<< /M 5 0 R /Contents <3082> >>\nendobj\n"),
            SignedM::Unreadable,
            "indirect /M"
        );
        let le: String = "D:20200101000000Z"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .map(|b| format!("{b:02X}"))
            .collect();
        assert_eq!(
            m(format!("1 0 obj\n<< /M <FFFE{le}> /Contents <3082> >>\nendobj\n").as_bytes()),
            ok
        );
        assert_eq!(
            m(b"1 0 obj\n<< /M (\xEF\xBB\xBFD:20200101000000Z) /Contents <3082> >>\nendobj\n"),
            ok
        );
        assert_eq!(
            m(b"1 0 obj\n<< /M (D:20200101000000Z) /Contents <3082> /N (endobj) >>\nendobj\n"),
            ok
        );
        assert_eq!(
            m(b"1 0 obj\r<< /M (D:20200101000000Z)\r/Contents <3082>\r>>\rendobj\r"),
            ok
        );
        assert_eq!(
            m(b"1 0 obj%x\n<< /M (D:20200101000000Z) /Contents <3082> >>\nendobj\n"),
            ok
        );
        // Bytes past what the signature covers are not read.
        let pdf: &[u8] = b"1 0 obj\n<< /Contents <3082>%%EOF\n/M (D:19990101000000Z) >>\nendobj\n";
        let (b, c) = gap(pdf);
        let src: &[u8] = pdf;
        assert_eq!(signed_m(&src, b, c, c + 6), SignedM::Unreadable);
        // A dictionary that starts more than the window before the gap.
        let mut big = b"1 0 obj\n<< /M (D:20200101000000Z) /Pad (".to_vec();
        big.extend(std::iter::repeat_n(b'a', 70_000));
        big.extend_from_slice(b") /Contents <3082> >>\nendobj\n");
        assert_eq!(m(&big), SignedM::Unreadable);
        let mut big2 = b"1 0 obj\n<< /M (D:20200101000000Z) /Pad (".to_vec();
        big2.extend(std::iter::repeat_n(b'a', 70_000));
        big2.extend_from_slice(b"2 0 obj <</M (D:19990101000000Z)) /Contents <3082> >>\nendobj\n");
        assert_eq!(m(&big2), SignedM::Unreadable);
    }

    #[test]
    fn pathological_inputs_stay_fast() {
        let start = std::time::Instant::now();
        let mut p = Vec::new();
        for _ in 0..21_000 {
            p.extend_from_slice(b"1 obj");
        }
        p.extend_from_slice(b"/Contents <00>>>");
        assert_eq!(m(&p), SignedM::Unreadable);
        let mut p = Vec::new();
        for _ in 0..30_000 {
            p.extend_from_slice(b" 1");
        }
        p.extend_from_slice(b" 0 obj<< /Contents <00> >>\nendobj");
        assert_eq!(m(&p), SignedM::Absent);
        let mut p = b"1 0 obj<<".to_vec();
        for _ in 0..30_000 {
            p.extend_from_slice(b"<<");
        }
        p.extend_from_slice(b"/Contents <00> >>");
        assert_eq!(m(&p), SignedM::Unreadable);
        let mut p = Vec::new();
        for _ in 0..6_000 {
            p.extend_from_slice(b"% 1 0 obj ");
        }
        p.extend_from_slice(b"/Contents <00> >>");
        assert_eq!(
            m(&p),
            SignedM::Unreadable,
            "every header on one commented line"
        );
        assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
    }

    #[test]
    fn utf16_and_hex_m() {
        let hex: String = "D:20260101000000Z"
            .encode_utf16()
            .map(|u| format!("{u:04X}"))
            .collect();
        let pdf = format!("5 0 obj<</M<FEFF{hex}>/Contents <00>>>endobj").into_bytes();
        assert_eq!(m(&pdf), text_m("D:20260101000000Z"));
    }
}
