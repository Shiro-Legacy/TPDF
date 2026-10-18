// SPDX-License-Identifier: MIT
//! The standard families a form's text is measured in, WinAnsiEncoding
//! (ISO 32000-2 Annex D) and PDF literal strings (spec 07 section 2.4).
//! Advances are the URW faces' (OFL) through `builtin_metrics`, by the
//! glyph names WinAnsiEncoding gives each code; Courier is 600 throughout.
use std::sync::OnceLock;

use crate::builtin_metrics::BuiltinFace;
use crate::builtin_outlines::{BuiltinOutline, BuiltinOutlineCommand};
use crate::document::Document;
use crate::object::{Dictionary, Object};

use super::scripts::Glyph;
use super::scripts::data;

/// A standard family, for advances when a font has no /Widths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Helvetica,
    HelveticaBold,
    Times,
    Courier,
    Symbol,
    ZapfDingbats,
}

const FAMILIES: [Family; 6] = [
    Family::Helvetica,
    Family::HelveticaBold,
    Family::Times,
    Family::Courier,
    Family::Symbol,
    Family::ZapfDingbats,
];

impl Family {
    /// The family a /BaseFont names: a six-letter subset tag and `+` are
    /// stripped; then, case-insensitively, contains "courier" → Courier,
    /// "times" → Times, "dingbats" or "zapf" → ZapfDingbats, equals
    /// "symbol" → Symbol, contains "bold" → HelveticaBold, else Helvetica.
    pub(crate) fn of(base: &[u8]) -> Self {
        let tagged =
            base.len() > 7 && base[6] == b'+' && base[..6].iter().all(|b| b.is_ascii_uppercase());
        let name = if tagged { &base[7..] } else { base };
        let lower = name.to_ascii_lowercase();
        let has = |needle: &[u8]| lower.windows(needle.len()).any(|w| w == needle);
        if has(b"courier") {
            Self::Courier
        } else if has(b"times") {
            Self::Times
        } else if has(b"dingbats") || has(b"zapf") {
            Self::ZapfDingbats
        } else if lower == b"symbol" {
            Self::Symbol
        } else if has(b"bold") {
            Self::HelveticaBold
        } else {
            Self::Helvetica
        }
    }

    /// Helvetica, Helvetica-Bold, Times-Roman, Courier, Symbol, ZapfDingbats.
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

    /// Symbol or ZapfDingbats.
    pub(crate) fn symbolic(self) -> bool {
        matches!(self, Self::Symbol | Self::ZapfDingbats)
    }

    /// The URW face measuring this family's glyphs by name; None for
    /// Courier (600 throughout) and the symbolic families.
    pub(crate) fn face(self) -> Option<BuiltinFace> {
        match self {
            Self::Helvetica => Some(BuiltinFace::Helvetica),
            Self::HelveticaBold => Some(BuiltinFace::HelveticaBold),
            Self::Times => Some(BuiltinFace::TimesRoman),
            Self::Courier | Self::Symbol | Self::ZapfDingbats => None,
        }
    }

    fn index(self) -> usize {
        FAMILIES.iter().position(|f| *f == self).unwrap_or(0)
    }

    /// The advance (1/1000 em) of the glyph `name` in this family: the
    /// URW face's; 600 for Courier; the Helvetica face's for a glyph the
    /// face lacks and for the symbolic families; 0 when no face has it.
    pub(crate) fn glyph_advance(self, name: &[u8]) -> f64 {
        if self == Self::Courier {
            return if BuiltinFace::Helvetica.lookup_glyph(name).is_some() {
                600.0
            } else {
                0.0
            };
        }
        let face = self.face().unwrap_or(BuiltinFace::Helvetica);
        face.lookup_glyph(name)
            .or_else(|| BuiltinFace::Helvetica.lookup_glyph(name))
            .map_or(0.0, |advance| advance.value())
    }

    /// The vertical extent of the family's printable ASCII glyphs, in em:
    /// (top, bottom), top > 0 > bottom. A line of text is placed by this
    /// glyph box. Courier, which has no outlines here, and the symbolic
    /// families take the union of the Helvetica and Times boxes.
    pub(crate) fn glyph_box(self) -> (f64, f64) {
        static BOXES: OnceLock<[(f64, f64); 3]> = OnceLock::new();
        let boxes = BOXES.get_or_init(|| {
            // The outlines are compiled in, so the fallback is never
            // used; it is a deliberately roomy box (a full em above, a
            // third below) so text placed by it stays inside a clip.
            let of = |face| ascii_box(face).unwrap_or((1.0, -0.33));
            [
                of(BuiltinFace::Helvetica),
                of(BuiltinFace::HelveticaBold),
                of(BuiltinFace::TimesRoman),
            ]
        });
        match self {
            Self::Helvetica => boxes[0],
            Self::HelveticaBold => boxes[1],
            Self::Times => boxes[2],
            _ => (boxes[0].0.max(boxes[2].0), boxes[0].1.min(boxes[2].1)),
        }
    }
}

/// The extent of every point of the printable ASCII glyphs of `face`.
fn ascii_box(face: BuiltinFace) -> Option<(f64, f64)> {
    let (mut top, mut bottom) = (i32::MIN, i32::MAX);
    for code in 0x21u8..=0x7E {
        let Some(outline) = win_ansi_name(code).and_then(|name| BuiltinOutline::lookup(face, name))
        else {
            continue;
        };
        for command in outline.commands() {
            let mut see = |y: i32| {
                top = top.max(y);
                bottom = bottom.min(y);
            };
            match *command {
                BuiltinOutlineCommand::MoveTo { y, .. }
                | BuiltinOutlineCommand::LineTo { y, .. } => see(y),
                BuiltinOutlineCommand::CubicTo { y1, y2, y, .. } => {
                    see(y1);
                    see(y2);
                    see(y);
                }
                BuiltinOutlineCommand::Close => {}
            }
        }
    }
    (top > 0 && bottom < 0).then(|| (f64::from(top) / 1000.0, f64::from(bottom) / 1000.0))
}

/// Advances of the 256 WinAnsi codes, 1/1000 em.
#[derive(Clone, Debug)]
pub(crate) struct Metrics {
    widths: Vec<f64>,
}

impl Metrics {
    /// The family's WinAnsi advances (URW data through `builtin_metrics`
    /// for Helvetica, Helvetica-Bold and Times; 600 for every Courier
    /// glyph; codes without a glyph 0). The symbolic families measure as
    /// Helvetica: no text is drawn in them through WinAnsi.
    pub(crate) fn family(family: Family) -> Self {
        static CACHE: [OnceLock<Metrics>; 6] = [
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
        ];
        CACHE[family.index()]
            .get_or_init(|| {
                let measured = if family.symbolic() {
                    Family::Helvetica
                } else {
                    family
                };
                let widths = (0..=255u8)
                    .map(|code| {
                        win_ansi_name(code).map_or(0.0, |name| measured.glyph_advance(name))
                    })
                    .collect();
                Metrics { widths }
            })
            .clone()
    }

    /// A simple font dictionary's /FirstChar + /Widths over its family's
    /// advances; codes outside /Widths take /FontDescriptor /MissingWidth
    /// (default 0); non-finite entries 0.
    pub(crate) fn font(doc: &Document<'_>, font: &Dictionary) -> Self {
        let base = doc.dict_get(font, b"BaseFont").as_name().unwrap_or(b"");
        let family = Family::of(base);
        let Some(widths) = doc.dict_get(font, b"Widths").as_array() else {
            return Self::family(family);
        };
        let descriptor = doc.dict_get(font, b"FontDescriptor").as_dict();
        let missing = descriptor
            .and_then(|d| doc.dict_get(d, b"MissingWidth").as_f64())
            .filter(|w| w.is_finite())
            .unwrap_or(0.0);
        let first = doc.dict_get(font, b"FirstChar").as_i64().unwrap_or(0);
        let mut out = vec![missing; 256];
        for (at, width) in widths.iter().enumerate().take(256) {
            let Ok(code) = usize::try_from(first.saturating_add(at as i64)) else {
                continue;
            };
            if let Some(slot) = out.get_mut(code) {
                *slot = doc
                    .resolve(width)
                    .as_f64()
                    .filter(|w| w.is_finite())
                    .unwrap_or(0.0);
            }
        }
        Self { widths: out }
    }

    #[allow(dead_code)] // spec 07 section 2.4 interface
    pub(crate) fn from_widths(widths: Vec<f64>) -> Self {
        let mut widths = widths;
        widths.truncate(256);
        for width in &mut widths {
            if !width.is_finite() {
                *width = 0.0;
            }
        }
        Self { widths }
    }

    pub(crate) fn advance(&self, byte: u8) -> f64 {
        self.widths.get(usize::from(byte)).copied().unwrap_or(0.0)
    }
}

/// The glyph name WinAnsiEncoding gives `code` (Annex D), None when the
/// code is not defined.
pub(crate) fn win_ansi_name(code: u8) -> Option<&'static [u8]> {
    data::WIN_ANSI
        .binary_search_by_key(&code, |entry| entry.0)
        .ok()
        .map(|at| data::WIN_ANSI[at].2)
}

/// WinAnsiEncoding code of `c` (Annex D); ASCII is itself; U+00A0 → 0xA0
/// and U+00AD → 0xAD.
pub(crate) fn win_ansi_code(c: char) -> Option<u8> {
    if c.is_ascii() {
        return Some(c as u8);
    }
    let value = u32::from(c);
    data::WIN_ANSI_BY_CHAR
        .binary_search_by_key(&value, |entry| entry.0)
        .ok()
        .map(|at| data::WIN_ANSI_BY_CHAR[at].1)
}

/// `text` as WinAnsi bytes, `?` for characters without a code.
#[allow(dead_code)] // spec 07 section 2.4 interface
pub(crate) fn win_ansi(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| win_ansi_code(c).unwrap_or(b'?'))
        .collect()
}

/// Advance of `glyphs` at size 1 (text space units: 1 is one em).
pub(crate) fn advance(glyphs: &[Glyph]) -> f64 {
    glyphs.iter().map(|g| g.advance).sum::<f64>() / 1000.0
}

/// A PDF literal string: `(`, `)`, `\` escaped, bytes outside 0x20..0x7E
/// as three-digit octal escapes.
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

/// The font dictionary `value` names, resolved.
pub(crate) fn font_dict<'d>(doc: &'d Document<'_>, value: &'d Object) -> Option<&'d Dictionary> {
    doc.resolve(value).as_dict()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_follow_their_base_font_names() {
        for (name, family) in [
            (&b"Helvetica"[..], Family::Helvetica),
            (b"ABCDEF+Arial-BoldMT", Family::HelveticaBold),
            (b"Times-Bold", Family::Times),
            (b"CourierNewPSMT", Family::Courier),
            (b"ZapfDingbats", Family::ZapfDingbats),
            (b"Symbol", Family::Symbol),
            (b"SymbolMT", Family::Helvetica),
            (b"abcdef+Courier", Family::Courier),
        ] {
            assert_eq!(
                Family::of(name),
                family,
                "{}",
                String::from_utf8_lossy(name)
            );
        }
    }

    #[test]
    fn win_ansi_maps_latin_and_replaces_the_rest() {
        assert_eq!(win_ansi("aé€日"), [b'a', 0xE9, 0x80, b'?']);
        assert_eq!(win_ansi_code('\u{A0}'), Some(0xA0));
        assert_eq!(win_ansi_code('\u{AD}'), Some(0xAD));
        assert_eq!(win_ansi_code('Ł'), None);
    }

    #[test]
    fn standard_advances_come_from_the_urw_faces() {
        let helvetica = Metrics::family(Family::Helvetica);
        assert_eq!(helvetica.advance(b'A'), 667.0);
        assert_eq!(helvetica.advance(b' '), 278.0);
        assert_eq!(helvetica.advance(0xA0), 278.0);
        assert_eq!(helvetica.advance(0x81), 0.0);
        assert_eq!(Metrics::family(Family::Times).advance(b'A'), 722.0);
        assert_eq!(Metrics::family(Family::Courier).advance(0xE9), 600.0);
        assert_eq!(Metrics::family(Family::Courier).advance(0x81), 0.0);
        let (top, bottom) = Family::Helvetica.glyph_box();
        assert!(top > 0.7 && top < 1.0 && bottom < -0.15 && bottom > -0.4);
    }

    #[test]
    fn literals_escape_what_they_must() {
        assert_eq!(literal(b"a(b)\\c\n\xE9"), "(a\\(b\\)\\\\c\\012\\351)");
    }
}
