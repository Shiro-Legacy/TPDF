// SPDX-License-Identifier: AGPL-3.0-or-later
//! The base-14 fonts annotation appearances write: WinAnsi encoding and
//! advances, measured from MuPDF 1.28.1's built-in fonts (the Adobe AFM
//! widths). Characters WinAnsi lacks have no width and draw as `?`.

/// A standard font an appearance names in its /Resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base14 {
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    HelveticaBoldOblique,
    TimesRoman,
    TimesBold,
    TimesItalic,
    TimesBoldItalic,
    Courier,
    CourierBold,
    CourierOblique,
    CourierBoldOblique,
}

impl Base14 {
    /// The font a default appearance's font name stands for (the AcroForm
    /// abbreviations MuPDF writes); anything else is Helvetica.
    pub fn from_resource_name(name: &[u8]) -> Self {
        match name {
            b"HeBo" | b"Helvetica-Bold" => Self::HelveticaBold,
            b"HeOb" | b"Helvetica-Oblique" => Self::HelveticaOblique,
            b"HeBO" | b"Helvetica-BoldOblique" => Self::HelveticaBoldOblique,
            b"TiRo" | b"Times-Roman" => Self::TimesRoman,
            b"TiBo" | b"Times-Bold" => Self::TimesBold,
            b"TiIt" | b"Times-Italic" => Self::TimesItalic,
            b"TiBI" | b"Times-BoldItalic" => Self::TimesBoldItalic,
            b"Cour" | b"Courier" => Self::Courier,
            b"CoBo" | b"Courier-Bold" => Self::CourierBold,
            b"CoOb" | b"Courier-Oblique" => Self::CourierOblique,
            b"CoBO" | b"Courier-BoldOblique" => Self::CourierBoldOblique,
            _ => Self::Helvetica,
        }
    }

    pub fn base_font(self) -> &'static [u8] {
        match self {
            Self::Helvetica => b"Helvetica",
            Self::HelveticaBold => b"Helvetica-Bold",
            Self::HelveticaOblique => b"Helvetica-Oblique",
            Self::HelveticaBoldOblique => b"Helvetica-BoldOblique",
            Self::TimesRoman => b"Times-Roman",
            Self::TimesBold => b"Times-Bold",
            Self::TimesItalic => b"Times-Italic",
            Self::TimesBoldItalic => b"Times-BoldItalic",
            Self::Courier => b"Courier",
            Self::CourierBold => b"Courier-Bold",
            Self::CourierOblique => b"Courier-Oblique",
            Self::CourierBoldOblique => b"Courier-BoldOblique",
        }
    }

    /// The advance of WinAnsi code `byte` in 1/1000 em. Bold and oblique
    /// Helvetica and the italic Times share their family's regular widths
    /// within a few units; Courier is 600 throughout.
    pub fn advance(self, byte: u8) -> f64 {
        let table = match self {
            Self::Courier | Self::CourierBold | Self::CourierOblique | Self::CourierBoldOblique => {
                return if byte >= 32 && byte != 0x7F && defined(byte) {
                    600.0
                } else {
                    0.0
                };
            }
            Self::TimesRoman | Self::TimesItalic => &TIMES_ROMAN,
            Self::TimesBold | Self::TimesBoldItalic => &TIMES_BOLD,
            _ => &HELVETICA,
        };
        usize::from(byte)
            .checked_sub(32)
            .and_then(|index| table.get(index))
            .map_or(0.0, |&width| f64::from(width))
    }

    /// The width of WinAnsi `bytes` at `size`.
    pub fn width(self, bytes: &[u8], size: f64) -> f64 {
        bytes.iter().map(|&b| self.advance(b)).sum::<f64>() * size / 1000.0
    }

    /// The simple font dictionary an appearance's /Resources holds.
    pub fn dictionary(self) -> crate::object::Dictionary {
        use crate::object::Object;
        let mut font = crate::object::Dictionary::new();
        font.insert(b"Type".to_vec(), Object::name("Font"));
        font.insert(b"Subtype".to_vec(), Object::name("Type1"));
        font.insert(b"BaseFont".to_vec(), Object::name(self.base_font()));
        font.insert(b"Encoding".to_vec(), Object::name("WinAnsiEncoding"));
        font
    }
}

/// Codes 0x80..=0x9F that WinAnsi defines.
fn defined(byte: u8) -> bool {
    !matches!(byte, 0x81 | 0x8D | 0x8F | 0x90 | 0x9D)
}

/// engine-core.ts `winAnsiByte`: the WinAnsi code of a character, if any.
pub fn win_ansi(c: char) -> Option<u8> {
    let code = u32::from(c);
    if (0x20..=0x7E).contains(&code) || (0xA0..=0xFF).contains(&code) {
        return u8::try_from(code).ok();
    }
    let byte = match code {
        0x20AC => 0x80,
        0x201A => 0x82,
        0x0192 => 0x83,
        0x201E => 0x84,
        0x2026 => 0x85,
        0x2020 => 0x86,
        0x2021 => 0x87,
        0x02C6 => 0x88,
        0x2030 => 0x89,
        0x0160 => 0x8A,
        0x2039 => 0x8B,
        0x0152 => 0x8C,
        0x017D => 0x8E,
        0x2018 => 0x91,
        0x2019 => 0x92,
        0x201C => 0x93,
        0x201D => 0x94,
        0x2022 => 0x95,
        0x2013 => 0x96,
        0x2014 => 0x97,
        0x02DC => 0x98,
        0x2122 => 0x99,
        0x0161 => 0x9A,
        0x203A => 0x9B,
        0x0153 => 0x9C,
        0x017E => 0x9E,
        0x0178 => 0x9F,
        _ => return None,
    };
    Some(byte)
}

/// Text as WinAnsi bytes: a control character stays as it is (MuPDF writes
/// a tab as a tab), any other character without a code becomes `?`.
pub fn encode(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match (win_ansi(c), u8::try_from(u32::from(c))) {
            (Some(byte), _) => byte,
            (None, Ok(byte)) if byte < 0x20 => byte,
            (None, _) => b'?',
        })
        .collect()
}

/// Times-Bold advances (1/1000 em) of WinAnsi codes 32..=255.
const TIMES_BOLD: [u16; 224] = [
    250, 333, 555, 500, 500, 1000, 833, 278, 333, 333, 500, 570, 250, 333, 250, 278, 500, 500, 500,
    500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500, 930, 722, 667, 722, 722, 667,
    611, 778, 778, 389, 500, 778, 667, 944, 722, 778, 611, 778, 722, 556, 667, 722, 722, 1000, 722,
    722, 667, 333, 278, 333, 581, 500, 333, 500, 556, 444, 556, 444, 333, 500, 556, 278, 333, 556,
    278, 833, 556, 500, 556, 556, 444, 389, 333, 556, 500, 722, 500, 500, 444, 394, 220, 394, 520,
    0, 500, 0, 333, 500, 500, 1000, 500, 500, 333, 1000, 556, 333, 1000, 0, 667, 0, 0, 333, 333,
    500, 500, 350, 500, 1000, 333, 1000, 389, 333, 722, 0, 444, 722, 250, 333, 500, 500, 500, 500,
    220, 500, 333, 747, 300, 500, 570, 333, 747, 333, 400, 570, 300, 300, 333, 556, 540, 250, 333,
    300, 330, 500, 750, 750, 750, 500, 722, 722, 722, 722, 722, 722, 1000, 722, 667, 667, 667, 667,
    389, 389, 389, 389, 722, 722, 778, 778, 778, 778, 778, 570, 778, 722, 722, 722, 722, 722, 611,
    556, 500, 500, 500, 500, 500, 500, 722, 444, 444, 444, 444, 444, 278, 278, 278, 278, 500, 556,
    500, 500, 500, 500, 500, 570, 500, 556, 556, 556, 556, 500, 556, 500,
];

/// Helvetica advances (1/1000 em) of WinAnsi codes 32..=255.
const HELVETICA: [u16; 224] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
    0, 556, 0, 222, 556, 333, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0, 0, 222, 222,
    333, 333, 350, 556, 1000, 333, 1000, 500, 333, 944, 0, 500, 667, 278, 333, 556, 556, 556, 556,
    260, 556, 333, 737, 370, 556, 584, 333, 737, 333, 400, 584, 333, 333, 333, 556, 537, 278, 333,
    333, 365, 556, 834, 834, 834, 611, 667, 667, 667, 667, 667, 667, 1000, 722, 667, 667, 667, 667,
    278, 278, 278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778, 722, 722, 722, 722, 667, 667,
    611, 556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278, 278, 556, 556,
    556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500,
];

/// Times-Roman advances (1/1000 em) of WinAnsi codes 32..=255.
const TIMES_ROMAN: [u16; 224] = [
    250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278, 500, 500, 500,
    500, 500, 500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444, 921, 722, 667, 667, 722, 611,
    556, 722, 722, 333, 389, 722, 611, 889, 722, 722, 556, 722, 667, 556, 611, 722, 722, 944, 722,
    722, 611, 333, 278, 333, 469, 500, 333, 444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500,
    278, 778, 500, 500, 500, 500, 333, 389, 278, 500, 500, 722, 500, 500, 444, 480, 200, 480, 541,
    0, 500, 0, 333, 500, 444, 1000, 500, 500, 333, 1000, 556, 333, 889, 0, 611, 0, 0, 333, 333,
    444, 444, 350, 500, 1000, 333, 980, 389, 333, 722, 0, 444, 722, 250, 333, 500, 500, 500, 500,
    200, 500, 333, 760, 276, 500, 564, 333, 760, 333, 400, 564, 300, 300, 333, 500, 453, 250, 333,
    300, 310, 500, 750, 750, 750, 444, 722, 722, 722, 722, 722, 722, 889, 667, 611, 611, 611, 611,
    333, 333, 333, 333, 722, 722, 722, 722, 722, 722, 722, 564, 722, 722, 722, 722, 722, 722, 556,
    500, 444, 444, 444, 444, 444, 444, 667, 444, 444, 444, 444, 444, 278, 278, 278, 278, 500, 500,
    500, 500, 500, 500, 500, 564, 500, 500, 500, 500, 500, 500, 500, 500,
];
