// SPDX-License-Identifier: MIT
//! The signature dictionary as signed: its `/M` (the signer's claimed time)
//! read from the bytes the signature covers, around the `/Contents` gap.
//!
//! A PAdES signature carries its time only in `/M`. A later incremental
//! update can redefine the signature dictionary (same `/ByteRange` and
//! `/Contents`, another `/M`), and a reader that takes the newest version
//! then reports a time nobody signed. The dictionary that holds the gap is,
//! byte for byte, the one in the signed revision, so its `/M` is the signed
//! one: it is read here with a small PDF tokenizer and nothing else is used.

use crate::source::Source;

/// How far before the gap the dictionary may start, and after it end.
const WINDOW: u64 = 64 * 1024;

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    DictOpen,
    DictClose,
    ArrayOpen,
    ArrayClose,
    Name(Vec<u8>),
    Str(Vec<u8>),
    /// A number, `R`, `true`, `null` or another bare word.
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

/// Tokens of `s` (`None` when a string or comment runs off the end, or a
/// character cannot start a token).
fn tokens(s: &[u8]) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let b = s[i];
        if is_white(b) {
            i += 1;
        } else if b == b'%' {
            while i < s.len() && s[i] != b'\n' && s[i] != b'\r' {
                i += 1;
            }
        } else if s[i..].starts_with(b"<<") {
            out.push(Tok::DictOpen);
            i += 2;
        } else if s[i..].starts_with(b">>") {
            out.push(Tok::DictClose);
            i += 2;
        } else if b == b'[' {
            out.push(Tok::ArrayOpen);
            i += 1;
        } else if b == b']' {
            out.push(Tok::ArrayClose);
            i += 1;
        } else if b == b'/' {
            let start = i + 1;
            i += 1;
            while i < s.len() && !is_white(s[i]) && !is_delim(s[i]) {
                i += 1;
            }
            out.push(Tok::Name(s[start..i].to_vec()));
        } else if b == b'<' {
            let mut digits = Vec::new();
            i += 1;
            loop {
                let c = *s.get(i)?;
                i += 1;
                if c == b'>' {
                    break;
                }
                if !is_white(c) {
                    digits.push(hexval(c)?);
                }
            }
            if digits.len() % 2 == 1 {
                digits.push(0);
            }
            out.push(Tok::Str(
                digits.chunks(2).map(|p| p[0] << 4 | p[1]).collect(),
            ));
        } else if b == b'(' {
            let mut depth = 1;
            let mut text = Vec::new();
            i += 1;
            loop {
                let c = *s.get(i)?;
                i += 1;
                match c {
                    b'\\' => {
                        let e = *s.get(i)?;
                        i += 1;
                        match e {
                            b'n' => text.push(b'\n'),
                            b'r' => text.push(b'\r'),
                            b't' => text.push(b'\t'),
                            b'b' => text.push(8),
                            b'f' => text.push(12),
                            b'0'..=b'7' => {
                                let mut v = u32::from(e - b'0');
                                for _ in 0..2 {
                                    match s.get(i) {
                                        Some(&d @ b'0'..=b'7') => {
                                            v = v * 8 + u32::from(d - b'0');
                                            i += 1;
                                        }
                                        _ => break,
                                    }
                                }
                                text.push((v & 0xff) as u8);
                            }
                            b'\r' => {
                                if s.get(i) == Some(&b'\n') {
                                    i += 1;
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
                            break;
                        }
                        text.push(c);
                    }
                    _ => text.push(c),
                }
            }
            out.push(Tok::Str(text));
        } else if is_delim(b) {
            return None;
        } else {
            let start = i;
            while i < s.len() && !is_white(s[i]) && !is_delim(s[i]) {
                i += 1;
            }
            out.push(Tok::Word(s[start..i].to_vec()));
        }
    }
    Some(out)
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
            (Tok::DictOpen | Tok::ArrayOpen, Some(_)) => self.depth = 1,
            (Tok::ArrayClose, Some(_)) => return false,
            (Tok::Str(s), Some(k)) => {
                if k == b"M" {
                    self.m = Some(s.clone());
                }
                self.key = None;
                self.words = 0;
            }
            (Tok::Word(_), Some(_)) => {
                self.key = None;
                self.words = 1;
            }
            (Tok::Name(_), Some(_)) => {
                self.key = None;
                self.words = 0;
            }
        }
        true
    }
}

/// A PDF text string as Rust text (UTF-16BE with a BOM, else bytes as Latin-1).
fn text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xfe, 0xff]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|&b| char::from(b)).collect()
    }
}

/// Where the dictionary holding the gap starts: just after the `N G obj`
/// that opens the last object before `b`.
fn object_start(before: &[u8]) -> Option<usize> {
    let mut at = before.len();
    while at >= 3 {
        let pos = before[..at].windows(3).rposition(|w| w == b"obj")?;
        let after_ok = before
            .get(pos + 3)
            .is_none_or(|&c| is_white(c) || c == b'<');
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
        if after_ok && fields == 2 && (j == 0 || is_white(head[j - 1])) {
            return Some(pos + 3);
        }
        at = pos;
    }
    None
}

/// The signed `/M` of the signature whose `/Contents` gap is `[b, c)` in
/// `src`: read from the dictionary that holds the gap, inside the bytes the
/// signature covers. `None` when that dictionary has no `/M` or cannot be
/// read.
pub fn signed_m(src: &dyn Source, b: u64, c: u64) -> Option<String> {
    let from = b.saturating_sub(WINDOW);
    let before = src.read_at(from, usize::try_from(b - from).ok()?)?;
    let after_len = src.len().saturating_sub(c).min(WINDOW);
    let after = src.read_at(c, usize::try_from(after_len).ok()?)?;
    let start = object_start(&before)?;
    let head = tokens(&before[start..])?;
    let mut walker = Walker::default();
    for t in &head {
        if !walker.feed(t) || walker.closed {
            return None;
        }
    }
    // The gap must be the value of /Contents at the dictionary's own level.
    if walker.depth != 0 || walker.key.as_deref() != Some(b"Contents".as_slice()) {
        return None;
    }
    walker.key = None;
    // The tail: tokenize up to `endobj` only (what follows is other objects).
    let end = after
        .windows(6)
        .position(|w| w == b"endobj")
        .unwrap_or(after.len());
    for t in &tokens(&after[..end])? {
        if !walker.feed(t) {
            return None;
        }
        if walker.closed {
            break;
        }
    }
    if !walker.closed {
        return None;
    }
    walker.m.map(|m| text(&m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gap(pdf: &[u8]) -> (u64, u64) {
        let b = pdf.windows(10).position(|w| w == b"/Contents ").unwrap() + 10;
        let c = b + pdf[b..].iter().position(|&x| x == b'>').unwrap() + 1;
        (b as u64, c as u64)
    }

    #[test]
    fn reads_the_m_of_the_dictionary_holding_the_gap() {
        let pdf: &[u8] = b"%PDF-1.7\n9 0 obj\n<</V 10 0 R/M(D:19990101000000Z)>>\nendobj\n10 0 obj\n<<\n/Prop_Build << /App << /Name /X >> >> /Reference [ << /P 2 >> ] /Ref 3 0 R\n/Contents <3082ABCD>\n/ByteRange [0 1 2 3] /M (D\\07220260916154752\\05504\\04700\\047) /Name (a\\)b) >>\nendobj\n11 0 obj << /M (D:20990101000000Z) >> endobj\n";
        let (b, c) = gap(pdf);
        let src: &[u8] = pdf;
        assert_eq!(
            signed_m(&src, b, c).as_deref(),
            Some("D:20260916154752-04'00'")
        );
    }

    #[test]
    fn a_gap_that_is_not_contents_or_no_m_gives_none() {
        let pdf: &[u8] = b"1 0 obj\n<< /Other <00> /Contents <3082> >>\nendobj\n";
        let b = pdf.windows(9).position(|w| w == b"/Other <0").unwrap() as u64 + 7;
        let src: &[u8] = pdf;
        assert_eq!(signed_m(&src, b, b + 4), None);
        let pdf: &[u8] = b"1 0 obj\n<< /Contents <3082> /Name (x) >>\nendobj\n";
        let (b, c) = gap(pdf);
        let src: &[u8] = pdf;
        assert_eq!(signed_m(&src, b, c), None);
        let pdf: &[u8] = b"1 0 obj\n<< /Contents <3082> /M (D:20260101000000Z)";
        let (b, c) = gap(pdf);
        let src: &[u8] = pdf;
        assert_eq!(signed_m(&src, b, c), None, "no closing >>");
    }

    #[test]
    fn utf16_and_hex_m() {
        let hex: String = "D:20260101000000Z"
            .encode_utf16()
            .map(|u| format!("{u:04X}"))
            .collect();
        let pdf = format!("5 0 obj<</M<FEFF{hex}>/Contents <00>>>endobj").into_bytes();
        let (b, c) = gap(&pdf);
        let src: &[u8] = &pdf;
        assert_eq!(signed_m(&src, b, c).as_deref(), Some("D:20260101000000Z"));
    }
}
