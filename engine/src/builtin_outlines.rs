// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exact-name, immutable unhinted outlines for explicitly selected pinned
//! Base14 substitutes. Generated font data retains its separate OFL license.
//! No runtime CFF parser, system font lookup, Unicode inversion or fallback.
use crate::builtin_metrics::BuiltinFace;
use std::{mem::size_of, ops::Range};
#[path = "builtin_outlines_data.rs"]
mod data;
#[path = "substitute_outlines_data.rs"]
mod substitutes;

/// Generator-enforced asset name bound; caller names may be arbitrary bytes.
pub const MAX_GLYPH_NAME_BYTES: usize = 128;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltinOutlineCommand {
    MoveTo {
        x: i32,
        y: i32,
    },
    LineTo {
        x: i32,
        y: i32,
    },
    CubicTo {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        x: i32,
        y: i32,
    },
    Close,
}
#[derive(Debug, Eq, PartialEq)]
pub struct BuiltinOutline {
    face: BuiltinFace,
    name: &'static [u8],
    native_gid: u16,
    command_start: u32,
    command_len: u32,
}
impl BuiltinOutline {
    fn table(face: BuiltinFace) -> &'static [Self] {
        match face {
            BuiltinFace::Helvetica => &data::HELVETICA,
            BuiltinFace::TimesRoman => &data::TIMES_ROMAN,
            BuiltinFace::HelveticaBold => &substitutes::HELVETICA_BOLD,
            BuiltinFace::HelveticaBoldOblique => &substitutes::HELVETICA_BOLD_OBLIQUE,
        }
    }
    /// Includes the explicit .notdef asset for inspection. PDF byte selection
    /// must refuse .notdef/undefined names in the document-bound loader.
    pub fn lookup(face: BuiltinFace, name: &[u8]) -> Option<&'static Self> {
        if name.len() > MAX_GLYPH_NAME_BYTES {
            return None;
        }
        let table = Self::table(face);
        let (mut low, mut high) = (0, table.len());
        // Owned binary search makes the comparison bound independent of a
        // standard-library implementation's exact comparison count.
        while low < high {
            let middle = low + (high - low) / 2;
            match table[middle].name.cmp(name) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => return Some(&table[middle]),
            }
        }
        None
    }
    pub fn glyphs(face: BuiltinFace) -> impl ExactSizeIterator<Item = &'static Self> {
        Self::table(face).iter()
    }
    /// Conservative logical upper bound for binary-search comparisons and
    /// compared name bytes, including failed lookups. G1 charges this before
    /// invoking lookup. Immutable table/name limits are generator-gated.
    pub fn lookup_work_bound(face: BuiltinFace) -> usize {
        let n = Self::table(face).len();
        (2 + (usize::BITS - n.leading_zeros()) as usize) * (1 + MAX_GLYPH_NAME_BYTES)
    }
    pub fn face(&self) -> BuiltinFace {
        self.face
    }
    pub fn name(&self) -> &'static [u8] {
        self.name
    }
    /// Asset-local native CFF glyph index, not a PDF CID or TrueType GlyphId.
    pub fn native_gid(&self) -> u16 {
        self.native_gid
    }
    pub fn units_per_em(&self) -> u16 {
        1000
    }
    pub fn asset_sha256(&self) -> &'static str {
        match self.face {
            BuiltinFace::Helvetica => {
                "366720e054067fa49d030f58005c739fc787dcc9727a9327a28767777791d706"
            }
            BuiltinFace::TimesRoman => {
                "eb609a306b81357853ba91c2a3c47f3bba04f3a8cfdeedc733b1aa4af5d03d4c"
            }
            BuiltinFace::HelveticaBold => {
                "d4ea80d1d0e29a1e950808009ca8f96fe07a0670b60b556f6b2cf0705337fa9f"
            }
            BuiltinFace::HelveticaBoldOblique => {
                "9cdfc2d0947339dce76f99bdf9e2d9fe01e472a384cf683007e6a0bb35634623"
            }
        }
    }
    pub fn command_range(&self) -> Range<usize> {
        self.command_start as usize..self.command_start as usize + self.command_len as usize
    }
    pub fn commands(&self) -> &'static [BuiltinOutlineCommand] {
        let commands: &'static [BuiltinOutlineCommand] = match self.face {
            BuiltinFace::Helvetica => &data::HELVETICA_COMMANDS,
            BuiltinFace::TimesRoman => &data::TIMES_ROMAN_COMMANDS,
            BuiltinFace::HelveticaBold => &substitutes::HELVETICA_BOLD_COMMANDS,
            BuiltinFace::HelveticaBoldOblique => &substitutes::HELVETICA_BOLD_OBLIQUE_COMMANDS,
        };
        &commands[self.command_range()]
    }
    /// Logical bytes in the immutable tables, commands and name payloads.
    /// Does not claim linker deduplication, object-file size or process RSS.
    pub fn static_bytes() -> usize {
        let records = data::HELVETICA.len()
            + data::TIMES_ROMAN.len()
            + substitutes::HELVETICA_BOLD.len()
            + substitutes::HELVETICA_BOLD_OBLIQUE.len();
        let commands = data::HELVETICA_COMMANDS.len()
            + data::TIMES_ROMAN_COMMANDS.len()
            + substitutes::HELVETICA_BOLD_COMMANDS.len()
            + substitutes::HELVETICA_BOLD_OBLIQUE_COMMANDS.len();
        records * size_of::<Self>()
            + commands * size_of::<BuiltinOutlineCommand>()
            + Self::glyphs(BuiltinFace::Helvetica)
                .chain(Self::glyphs(BuiltinFace::TimesRoman))
                .chain(Self::glyphs(BuiltinFace::HelveticaBold))
                .chain(Self::glyphs(BuiltinFace::HelveticaBoldOblique))
                .map(|r| r.name.len())
                .sum::<usize>()
    }
}
