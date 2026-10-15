// SPDX-License-Identifier: AGPL-3.0-or-later
//! Text for synthesized appearances: WinAnsi bytes, a simple font's
//! advances, MuPDF's line breaking, and its `%g` number format.
use std::sync::OnceLock;

use crate::builtin_metrics::{BuiltinFace, BuiltinMetricLookup, BuiltinMetrics};
use crate::document::Document;
use crate::object::{Dictionary, Object};
use crate::simple_encoding::{GlyphUnicode, NamedEncoding, SimpleEncoding};

/// MuPDF's `%g` in `fz_append_printf`: the shortest digits of the single
/// precision value, never an exponent, no leading zero before the point.
pub(crate) fn num(value: f64) -> String {
    let max = f64::from(f32::MAX);
    let single = if value.is_nan() {
        0.0
    } else {
        value.clamp(-max, max) as f32
    };
    if single == 0.0 {
        return "0".into();
    }
    let text = format!("{}", single.abs());
    let text = match text.strip_prefix("0.") {
        Some(rest) => format!(".{rest}"),
        None => text,
    };
    if single < 0.0 {
        format!("-{text}")
    } else {
        text
    }
}

/// Unicode to WinAnsiEncoding bytes; ASCII is itself and anything the
/// encoding lacks becomes `?`.
pub(crate) fn win_ansi(text: &str) -> Vec<u8> {
    let table = win_ansi_table();
    text.chars()
        .map(|c| match u8::try_from(u32::from(c)) {
            Ok(byte) if byte < 0x80 => byte,
            _ => (table.binary_search_by(|(key, _)| key.cmp(&c)))
                .ok()
                .and_then(|index| table.get(index))
                .map_or(b'?', |(_, byte)| *byte),
        })
        .collect()
}

fn win_ansi_table() -> &'static [(char, u8)] {
    static TABLE: OnceLock<Vec<(char, u8)>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
        let mut table: Vec<(char, u8)> = (0x80..=0xFFu8)
            .filter_map(|code| match encoding.lookup_unicode(code) {
                GlyphUnicode::Unicode([c]) => Some((*c, code)),
                _ => None,
            })
            .collect();
        table.sort_unstable();
        table.dedup_by_key(|(c, _)| *c);
        table
    })
}

/// A base 14 family, for advances when a font has no /Widths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Family {
    Helvetica,
    HelveticaBold,
    Times,
    Courier,
    Symbol,
    ZapfDingbats,
}

impl Family {
    /// The family a /BaseFont names, by the usual substitutions; Helvetica
    /// for anything else, as MuPDF falls back.
    pub(crate) fn of(base: &[u8]) -> Self {
        let base = match base.iter().position(|&b| b == b'+') {
            Some(6) => base.get(7..).unwrap_or(base),
            _ => base,
        };
        let lower = String::from_utf8_lossy(base).to_ascii_lowercase();
        if lower.contains("courier") {
            Self::Courier
        } else if lower.contains("times") {
            Self::Times
        } else if lower.contains("dingbats") || lower.contains("zapf") {
            Self::ZapfDingbats
        } else if lower == "symbol" {
            Self::Symbol
        } else if lower.contains("bold") {
            Self::HelveticaBold
        } else {
            Self::Helvetica
        }
    }

    /// The standard /BaseFont of a font made for this family.
    pub(crate) fn base_font(self) -> &'static [u8] {
        match self {
            Self::Helvetica => b"Helvetica",
            Self::HelveticaBold => b"Helvetica-Bold",
            Self::Times => b"Times-Roman",
            Self::Courier => b"Courier",
            Self::Symbol => b"Symbol",
            Self::ZapfDingbats => b"ZapfDingbats",
        }
    }

    pub(crate) fn symbolic(self) -> bool {
        matches!(self, Self::Symbol | Self::ZapfDingbats)
    }
}

/// Advances of the 256 WinAnsi codes, in thousandths of the font size.
#[derive(Clone, Debug)]
pub(crate) struct Metrics {
    widths: Vec<f64>,
}

impl Metrics {
    /// A base 14 family's advances in WinAnsiEncoding.
    pub(crate) fn family(family: Family) -> Self {
        static CACHE: OnceLock<[Vec<f64>; 3]> = OnceLock::new();
        let [helvetica, bold, times] = CACHE.get_or_init(|| {
            let table = |face| {
                let metrics = BuiltinMetrics::from_encoding(
                    face,
                    SimpleEncoding::from_named(NamedEncoding::WinAnsi),
                );
                (0..=255u8)
                    .map(|code| match metrics.lookup(code) {
                        BuiltinMetricLookup::Advance(advance) => advance.value(),
                        _ => 0.0,
                    })
                    .collect()
            };
            [
                table(BuiltinFace::Helvetica),
                table(BuiltinFace::HelveticaBold),
                table(BuiltinFace::TimesRoman),
            ]
        });
        let widths = match family {
            Family::HelveticaBold => bold.clone(),
            Family::Times => times.clone(),
            Family::Courier => {
                let defined = |code: usize| code >= 0x20 && code != 0x7F;
                (0..256)
                    .map(|code| if defined(code) { 600.0 } else { 0.0 })
                    .collect()
            }
            _ => helvetica.clone(),
        };
        Self { widths }
    }

    /// A simple font's /Widths over its base family's advances.
    pub(crate) fn font(doc: &Document<'_>, font: &Dictionary) -> Self {
        let base = doc.dict_get(font, b"BaseFont").as_name().unwrap_or(b"");
        let mut metrics = Self::family(Family::of(base));
        let first = doc.dict_get(font, b"FirstChar").as_i64();
        let widths = doc.dict_get(font, b"Widths").as_array();
        if let (Some(first), Some(widths)) = (first, widths) {
            let missing = doc
                .dict_get(font, b"FontDescriptor")
                .as_dict()
                .and_then(|descriptor| doc.dict_get(descriptor, b"MissingWidth").as_f64())
                .unwrap_or(0.0);
            for (code, slot) in metrics.widths.iter_mut().enumerate() {
                let index = (code as i64)
                    .checked_sub(first)
                    .and_then(|i| usize::try_from(i).ok());
                *slot = match index.and_then(|index| widths.get(index)) {
                    Some(width) => doc.resolve(width).as_f64().unwrap_or(missing),
                    None => missing,
                };
                if !slot.is_finite() {
                    *slot = 0.0;
                }
            }
        }
        metrics
    }

    /// The advance of `bytes` at size 1.
    pub(crate) fn width(&self, bytes: &[u8]) -> f64 {
        bytes
            .iter()
            .map(|&b| self.widths.get(usize::from(b)).copied().unwrap_or(0.0))
            .sum::<f64>()
            / 1000.0
    }

    /// One code's advance in thousandths.
    pub(crate) fn advance(&self, byte: u8) -> f64 {
        self.widths.get(usize::from(byte)).copied().unwrap_or(0.0)
    }
}

/// One laid-out line: its bytes (a break's trailing space included) and the
/// width alignment uses (trailing spaces left out), at size 1.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub bytes: Vec<u8>,
    pub width: f64,
}

/// MuPDF's multiline layout: every CR or LF ends a line, and a line that
/// passes `max_width` (at `size`) breaks after its last space, the space
/// kept; a word wider than the line is never split. `limit` caps the lines.
pub(crate) fn break_lines(
    metrics: &Metrics,
    bytes: &[u8],
    size: f64,
    max_width: f64,
    limit: usize,
) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() && lines.len() < limit {
        let mut x = 0.0;
        let mut space = None;
        let mut end = rest.len();
        for (at, &byte) in rest.iter().enumerate() {
            if byte == b'\r' || byte == b'\n' {
                end = at;
                break;
            }
            if byte == b' ' {
                space = Some(at + 1);
            }
            x += metrics.advance(byte) * size / 1000.0;
            if let Some(space) = space
                && x > max_width
            {
                end = space;
                break;
            }
        }
        let (line, tail) = rest.split_at(end.min(rest.len()));
        let trimmed = line.len() - line.iter().rev().take_while(|&&b| b == b' ').count();
        lines.push(Line {
            bytes: line.to_vec(),
            width: metrics.width(line.get(..trimmed).unwrap_or(line)),
        });
        rest = match tail.first() {
            Some(b'\r' | b'\n') => tail.get(1..).unwrap_or(&[]),
            _ => tail,
        };
    }
    lines
}

/// A PDF literal string of `bytes` (MuPDF's `write_simple_string`): `(`,
/// `)` and `\` escaped, and bytes outside printable ASCII as octal escapes
/// where MuPDF writes them raw, so the content stays ASCII.
pub(crate) fn literal(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('(');
    for &byte in bytes {
        match byte {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(char::from(byte));
            }
            0x20..=0x7E => out.push(char::from(byte)),
            _ => out.push_str(&format!("\\{byte:03o}")),
        }
    }
    out.push(')');
    out
}

/// The resolved font dictionary `value` names, if any.
pub(crate) fn font_dict<'d>(doc: &'d Document<'_>, value: &'d Object) -> Option<&'d Dictionary> {
    doc.resolve(value).as_dict()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_as_mupdf_does() {
        assert_eq!(num(0.5), ".5");
        assert_eq!(num(-0.5), "-.5");
        assert_eq!(num(179.0), "179");
        assert_eq!(num(13.392), "13.392");
        assert_eq!(num(0.0), "0");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(1e-7), ".0000001");
    }

    #[test]
    fn lines_break_after_spaces_and_at_line_ends() {
        let metrics = Metrics::family(Family::Helvetica);
        let text = b"line two is a much longer line that should wrap around";
        let lines = break_lines(&metrics, text, 12.0, 176.0, 100);
        let texts: Vec<&[u8]> = lines.iter().map(|l| l.bytes.as_slice()).collect();
        assert_eq!(
            texts,
            [
                b"line two is a much longer line ".as_slice(),
                b"that should wrap around"
            ]
        );
        let lines = break_lines(&metrics, b"a\r\nb\rc", 10.0, 176.0, 100);
        let texts: Vec<&[u8]> = lines.iter().map(|l| l.bytes.as_slice()).collect();
        assert_eq!(texts, [b"a".as_slice(), b"", b"b", b"c"]);
        let lines = break_lines(&metrics, b"Supercalifragilistic word", 12.0, 56.0, 100);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].bytes, b"Supercalifragilistic ");
    }

    #[test]
    fn win_ansi_maps_latin_and_replaces_the_rest() {
        assert_eq!(win_ansi("aé€日"), vec![b'a', 0xE9, 0x80, b'?']);
    }
}
