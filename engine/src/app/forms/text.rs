// SPDX-License-Identifier: AGPL-3.0-or-later
//! Text for synthesized appearances: WinAnsi bytes, a simple font's
//! advances, MuPDF's line breaking, and its `%g` number format.
use std::sync::OnceLock;

use crate::builtin_metrics::{BuiltinFace, BuiltinMetricLookup, BuiltinMetrics};
use crate::document::Document;
use crate::object::{Dictionary, Object};
use crate::simple_encoding::{GlyphUnicode, NamedEncoding, SimpleEncoding};

use super::scripts::Glyph;

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
    text.chars()
        .map(|c| win_ansi_code(c).unwrap_or(b'?'))
        .collect()
}

/// A character's WinAnsiEncoding code; ASCII is itself.
pub(crate) fn win_ansi_code(c: char) -> Option<u8> {
    match u8::try_from(u32::from(c)) {
        Ok(byte) if byte < 0x80 => Some(byte),
        _ => {
            let table = win_ansi_table();
            (table.binary_search_by(|(key, _)| key.cmp(&c)))
                .ok()
                .and_then(|index| table.get(index))
                .map(|(_, byte)| *byte)
        }
    }
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
pub enum Family {
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

    /// One code's advance in thousandths.
    pub(crate) fn advance(&self, byte: u8) -> f64 {
        self.widths.get(usize::from(byte)).copied().unwrap_or(0.0)
    }
}

/// The advance of `glyphs` at size 1.
pub(crate) fn advance(glyphs: &[Glyph]) -> f64 {
    glyphs.iter().map(|g| g.advance).sum::<f64>() / 1000.0
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
    use super::super::scripts::{Cjk, Fonts, lines};
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
        let fonts = Fonts {
            latin: &metrics,
            family: Family::Helvetica,
            han: Cjk::Song,
        };
        let lines_of = |text: &str, size: f64, width: f64| -> Vec<String> {
            let (lines, _) = lines(text, &fonts, size, width, 100);
            (lines.iter())
                .map(|line| line.glyphs.iter().map(|g| g.ch).collect())
                .collect()
        };
        assert_eq!(
            lines_of(
                "line two is a much longer line that should wrap around",
                12.0,
                176.0
            ),
            ["line two is a much longer line ", "that should wrap around"]
        );
        assert_eq!(lines_of("a\r\nb\rc", 10.0, 176.0), ["a", "", "b", "c"]);
        assert_eq!(
            lines_of("Supercalifragilistic word", 12.0, 56.0),
            ["Supercalifragilistic ", "word"]
        );
    }

    #[test]
    fn win_ansi_maps_latin_and_replaces_the_rest() {
        assert_eq!(win_ansi("aé€日"), vec![b'a', 0xE9, 0x80, b'?']);
    }
}
