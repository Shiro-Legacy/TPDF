// SPDX-License-Identifier: AGPL-3.0-or-later
//! The font programs `scan-compose` embeds (app/ops/scan.rs): a subset of
//! the installed CJK fallback font (TrueType, for a CIDFontType2) and
//! subsets of the URW Nimbus Sans outlines the engine carries (bare CFF,
//! for an embedded Type1C simple font). A child of the view's fonts, so it
//! reuses their sfnt reader and writer, cmap reading and CFF writer rather
//! than a second copy of each.
use super::{
    CJK, CJK_REFUSED, CJK_WANTED, CffExtras, Glyph, Ordering, PoisonError, Sfnt, bmp_mapping,
    cff_program, glyph_data, glyphs_of, i16_at, own_metric, sfnt, u16_at, ucs4_runs,
};
use crate::builtin_metrics::BuiltinFace;
use hayro_interpret::font::FontData;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The installed CJK fallback font. When none is installed (and the last
/// install was not refused) `cjk_font_wanted` turns true, the host's cue
/// to install it and dispatch again, as a render's.
pub(crate) fn cjk_font() -> Option<FontData> {
    let installed = CJK.lock().unwrap_or_else(PoisonError::into_inner);
    match installed.as_ref() {
        Some(installed) => Some(installed.font.clone()),
        None => {
            if !CJK_REFUSED.load(Ordering::Relaxed) {
                CJK_WANTED.store(true, Ordering::Relaxed);
            }
            None
        }
    }
}

/// JavaScript's `Math.round`: halves round up.
fn js_round(value: f64) -> i64 {
    (value + 0.5).floor() as i64
}

/// A TrueType font read for subsetting: glyph lookup, advances and the
/// descriptor metrics, all in 1/1000 em.
pub(crate) struct TrueType<'a> {
    font: Sfnt<'a>,
    head: &'a [u8],
    hhea: &'a [u8],
    maxp: &'a [u8],
    hmtx: &'a [u8],
    loca: &'a [u8],
    glyf: &'a [u8],
    glyphs: u16,
    units: f64,
    long_loca: bool,
    long_metrics: u16,
    last: u16,
    bmp: Vec<u16>,
    planes: Vec<(u32, u32, u16)>,
}

/// A font's descriptor values in 1/1000 em (rounded as engine.ts rounds).
#[derive(Clone, Copy, Debug)]
pub(crate) struct DescriptorMetrics {
    pub ascent: i64,
    pub descent: i64,
    pub cap_height: i64,
    pub bbox: [i64; 4],
}

impl<'a> TrueType<'a> {
    /// None for a font that is not TrueType (glyf outlines) or lacks a
    /// table a subset needs.
    pub(crate) fn read(bytes: &'a [u8]) -> Option<Self> {
        let font = Sfnt::read(bytes)?;
        let (head, hhea, maxp, hmtx, loca, glyf, cmap) = (
            font.table(b"head")?,
            font.table(b"hhea")?,
            font.table(b"maxp")?,
            font.table(b"hmtx")?,
            font.table(b"loca")?,
            font.table(b"glyf")?,
            font.table(b"cmap")?,
        );
        let glyphs = u16_at(maxp, 4)?;
        let long_metrics = u16_at(hhea, 34)?.min(glyphs);
        let last = match long_metrics.checked_sub(1) {
            Some(i) => u16_at(hmtx, 4 * usize::from(i))?,
            None => 0,
        };
        let mut bmp = bmp_mapping(cmap)?;
        for gid in bmp.iter_mut().filter(|gid| **gid >= glyphs) {
            *gid = 0;
        }
        Some(Self {
            planes: ucs4_runs(cmap, glyphs).unwrap_or_default(),
            units: f64::from(u16_at(head, 18)?.clamp(16, 16_384)),
            long_loca: u16_at(head, 50)? == 1,
            font,
            head,
            hhea,
            maxp,
            hmtx,
            loca,
            glyf,
            glyphs,
            long_metrics,
            last,
            bmp,
        })
    }

    /// The glyph for `c`; 0 (.notdef) when the font has none.
    pub(crate) fn glyph(&self, c: char) -> u16 {
        let c = u32::from(c);
        if let Some(&gid) = self.bmp.get(c as usize) {
            return gid;
        }
        let run = self.planes.partition_point(|&(_, last, _)| last < c);
        (self.planes.get(run))
            .filter(|run| run.0 <= c)
            .and_then(|&(first, _, gid)| gid.checked_add(u16::try_from(c - first).ok()?))
            .filter(|&gid| gid < self.glyphs)
            .unwrap_or(0)
    }

    /// A glyph's advance in 1/1000 em.
    pub(crate) fn advance(&self, glyph: u16) -> f64 {
        let glyph = if glyph < self.glyphs { glyph } else { 0 };
        let (advance, _) = own_metric(self.hmtx, self.long_metrics, self.last, glyph);
        f64::from(advance) * 1000.0 / self.units
    }

    /// Ascent and descent (hhea), cap height (OS/2 version 2 and later,
    /// else 70% of the ascent) and bounding box (head).
    pub(crate) fn metrics(&self) -> DescriptorMetrics {
        let scale = |v: i16| js_round(f64::from(v) * 1000.0 / self.units);
        let ascent = i16_at(self.hhea, 4).unwrap_or(0);
        let cap = (self.font.table(b"OS/2"))
            .filter(|os2| u16_at(os2, 0).is_some_and(|version| version >= 2))
            .and_then(|os2| i16_at(os2, 88))
            .map_or_else(|| js_round(f64::from(ascent) * 0.7), i64::from);
        let bbox = [36, 38, 40, 42].map(|at| scale(i16_at(self.head, at).unwrap_or(0)));
        DescriptorMetrics {
            ascent: scale(ascent),
            descent: scale(i16_at(self.hhea, 6).unwrap_or(0)),
            cap_height: js_round(cap as f64 * 1000.0 / self.units),
            bbox,
        }
    }

    /// A font program whose glyph `i` draws this font's glyph `glyphs[i]`
    /// (`glyphs[0]` should be 0, .notdef), for a CIDFontType2 with an
    /// Identity CIDToGIDMap (so no cmap). Composite glyphs keep their
    /// instructions: their components join the subset after `glyphs` and
    /// are renumbered. A glyph that repeats an earlier one is a one-component
    /// composite of it (16 bytes), so many characters the font lacks cost
    /// little. Kept: each glyph's advance and side bearing, the hinting
    /// tables (cvt, fpgm, prep), and the name table's copyright and licence
    /// records.
    pub(crate) fn subset(&self, glyphs: &[u16]) -> Result<Vec<u8>, SubsetError> {
        let valid = |glyph: u16| if glyph < self.glyphs { glyph } else { 0 };
        let mut order: Vec<u16> = glyphs.iter().map(|&g| valid(g)).collect();
        let mut first: HashMap<u16, u16> = HashMap::new();
        let position = |i: usize| u16::try_from(i).map_err(|_| SubsetError::TooManyGlyphs);
        for (i, &glyph) in order.iter().enumerate() {
            first.entry(glyph).or_insert(position(i)?);
        }
        // Components, breadth first, each once; a cycle ends at a glyph
        // already in the subset.
        let mut at = 0;
        while at < order.len() {
            let bytes = glyph_data(self.glyf, self.loca, self.long_loca, order[at]);
            for component in components(bytes) {
                let component = valid(component);
                if let std::collections::hash_map::Entry::Vacant(slot) = first.entry(component) {
                    slot.insert(position(order.len())?);
                    order.push(component);
                }
            }
            at += 1;
        }
        let count = position(order.len())?;
        let budget = self
            .glyf
            .len()
            .saturating_add(16 * order.len())
            .saturating_add(1 << 16);
        let mut data: Vec<u8> = Vec::new();
        let mut offsets: Vec<u8> = Vec::with_capacity(4 * (order.len() + 1));
        let mut horizontal: Vec<u8> = Vec::with_capacity(4 * order.len());
        let (mut widest, mut repeats) = (0u16, false);
        for (i, &glyph) in order.iter().enumerate() {
            offsets.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let bytes = glyph_data(self.glyf, self.loca, self.long_loca, glyph);
            let earlier = first.get(&glyph).copied().filter(|&p| usize::from(p) != i);
            match earlier {
                Some(p) if bytes.len() >= 10 => {
                    // One component, glyph p at (0, 0), with its metrics.
                    repeats = true;
                    data.extend_from_slice(&(-1i16).to_be_bytes());
                    data.extend_from_slice(&bytes[2..10]);
                    data.extend_from_slice(&0x0202u16.to_be_bytes());
                    data.extend_from_slice(&p.to_be_bytes());
                    data.extend_from_slice(&[0, 0]);
                }
                Some(_) => {}
                None if bytes.first().is_some_and(|n| n & 0x80 != 0) => {
                    let start = data.len();
                    data.extend_from_slice(bytes);
                    renumber(&mut data[start..], |g| {
                        first.get(&valid(g)).copied().unwrap_or(0)
                    });
                }
                None => data.extend_from_slice(bytes),
            }
            let (advance, bearing) = own_metric(self.hmtx, self.long_metrics, self.last, glyph);
            widest = widest.max(advance);
            horizontal.extend_from_slice(&advance.to_be_bytes());
            horizontal.extend_from_slice(&bearing);
            data.resize(data.len().next_multiple_of(4), 0);
            if data.len() > budget || u32::try_from(data.len()).is_err() {
                return Err(SubsetError::TooLarge);
            }
        }
        offsets.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let unreadable = SubsetError::Unreadable;
        let mut head = self.head.get(..54).ok_or(unreadable)?.to_vec();
        head[8..12].copy_from_slice(&[0; 4]);
        head[50..52].copy_from_slice(&1u16.to_be_bytes());
        let mut hhea = self.hhea.get(..36).ok_or(unreadable)?.to_vec();
        hhea[10..12].copy_from_slice(&widest.to_be_bytes());
        hhea[34..36].copy_from_slice(&count.to_be_bytes());
        let mut maxp = self.maxp.to_vec();
        maxp.get_mut(4..6)
            .ok_or(unreadable)?
            .copy_from_slice(&count.to_be_bytes());
        if repeats && maxp.len() >= 32 {
            // maxComponentElements at least 1, maxComponentDepth one more.
            let elements = u16_at(&maxp, 28).unwrap_or(0).max(1);
            let depth = u16_at(&maxp, 30).unwrap_or(0).saturating_add(1);
            maxp[28..30].copy_from_slice(&elements.to_be_bytes());
            maxp[30..32].copy_from_slice(&depth.to_be_bytes());
        }
        // post 3.0: no glyph names (they would be by the font's glyph ids).
        let mut post = vec![0u8; 32];
        if let Some(own) = self.font.table(b"post").and_then(|post| post.get(..32)) {
            post.copy_from_slice(own);
        }
        post[..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
        let name = self.font.table(b"name").and_then(licence_names);
        let mut tables: Vec<([u8; 4], &[u8])> = vec![
            (*b"glyf", &data),
            (*b"head", &head),
            (*b"hhea", &hhea),
            (*b"hmtx", &horizontal),
            (*b"loca", &offsets),
            (*b"maxp", &maxp),
            (*b"post", &post),
        ];
        if let Some(name) = &name {
            tables.push((*b"name", name));
        }
        for tag in [b"cvt ", b"fpgm", b"prep"] {
            if let Some(table) = self.font.table(tag) {
                tables.push((*tag, table));
            }
        }
        tables.sort_by_key(|(tag, _)| *tag);
        let mut out = sfnt(*b"\0\x01\0\0", &tables).ok_or(SubsetError::TooLarge)?;
        checksum_adjustment(&mut out);
        Ok(out)
    }
}

/// Why a TrueType subset was not built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SubsetError {
    /// More than 65,535 glyphs with the composites' components.
    TooManyGlyphs,
    /// Glyph data past the font's own plus 16 bytes a glyph.
    TooLarge,
    /// A table too short to copy.
    Unreadable,
}

/// The offsets, in a composite glyph, of each component's glyph index.
fn component_offsets(glyph: &[u8]) -> Vec<usize> {
    let mut found = Vec::new();
    if !glyph.first().is_some_and(|n| n & 0x80 != 0) {
        return found;
    }
    let mut at = 10;
    while let Some(flags) = u16_at(glyph, at) {
        if at + 4 > glyph.len() {
            break;
        }
        found.push(at + 2);
        at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
        at += match flags {
            f if f & 0x0008 != 0 => 2,
            f if f & 0x0040 != 0 => 4,
            f if f & 0x0080 != 0 => 8,
            _ => 0,
        };
        if flags & 0x0020 == 0 || found.len() > 1024 {
            break;
        }
    }
    found
}

/// A composite glyph's components.
fn components(glyph: &[u8]) -> Vec<u16> {
    (component_offsets(glyph).into_iter())
        .filter_map(|at| u16_at(glyph, at))
        .collect()
}

/// Rewrites a composite glyph's component indices through `to`.
fn renumber(glyph: &mut [u8], to: impl Fn(u16) -> u16) {
    for at in component_offsets(glyph) {
        if let Some(old) = u16_at(glyph, at) {
            glyph[at..at + 2].copy_from_slice(&to(old).to_be_bytes());
        }
    }
}

/// A name table (format 0) of the copyright (0), licence (13) and licence
/// URL (14) records of `name`; None when it has none.
fn licence_names(name: &[u8]) -> Option<Vec<u8>> {
    let count = usize::from(u16_at(name, 2)?);
    let storage = usize::from(u16_at(name, 4)?);
    let mut records = Vec::new();
    let mut strings: Vec<u8> = Vec::new();
    for i in 0..count {
        let record = name.get(6 + 12 * i..18 + 12 * i)?;
        if !matches!(u16_at(record, 6), Some(0 | 13 | 14)) {
            continue;
        }
        let (length, offset) = (
            usize::from(u16_at(record, 8)?),
            usize::from(u16_at(record, 10)?),
        );
        let text = name.get(storage + offset..storage + offset + length)?;
        let mut kept = record.to_vec();
        kept[10..12].copy_from_slice(&u16::try_from(strings.len()).ok()?.to_be_bytes());
        strings.extend_from_slice(text);
        records.push(kept);
    }
    if records.is_empty() || strings.len() > 0xFFFF {
        return None;
    }
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&u16::try_from(records.len()).ok()?.to_be_bytes());
    out.extend_from_slice(&u16::try_from(6 + 12 * records.len()).ok()?.to_be_bytes());
    for record in records {
        out.extend_from_slice(&record);
    }
    out.extend_from_slice(&strings);
    Some(out)
}

/// head's checkSumAdjustment for the whole file (OpenType `head`).
fn checksum_adjustment(font: &mut [u8]) {
    let sum = (font.chunks(4)).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    });
    let count = usize::from(u16_at(font, 4).unwrap_or(0));
    let head = (0..count).find_map(|i| {
        let record = 12 + 16 * i;
        (font.get(record..record + 4) == Some(b"head"))
            .then(|| super::u32_at(font, record + 8))
            .flatten()
    });
    let Some(at) = head.and_then(|at| usize::try_from(at).ok()) else {
        return;
    };
    if let Some(slot) = font.get_mut(at + 8..at + 12) {
        slot.copy_from_slice(&0xB1B0_AFBAu32.wrapping_sub(sum).to_be_bytes());
    }
}

/// The copyright of URW's Nimbus Sans (engine/reference/licenses/urw-outlines/OFL.txt).
pub(crate) const NIMBUS_COPYRIGHT: &[u8] = b"Copyright 2016 by (URW)++ Design & Development.";
/// The licence the outlines and every subset of them are under; OFL.txt
/// names no Reserved Font Name, so a subset may keep the font's name.
pub(crate) const NIMBUS_NOTICE: &[u8] =
    b"Nimbus Sans, subset. Licensed under the SIL Open Font License, Version 1.1.";

/// The URW Nimbus Sans outlines of one face, by character.
pub(crate) struct Outlines {
    glyphs: Vec<Glyph>,
    by_char: HashMap<u32, usize>,
}

/// Nimbus Sans Regular (`bold` false) or Bold, built once per process.
pub(crate) fn outlines(bold: bool) -> &'static Outlines {
    static FACES: [OnceLock<Outlines>; 2] = [OnceLock::new(), OnceLock::new()];
    FACES[usize::from(bold)].get_or_init(|| {
        let face = if bold {
            BuiltinFace::HelveticaBold
        } else {
            BuiltinFace::Helvetica
        };
        let glyphs = glyphs_of(face);
        let mut by_char = HashMap::new();
        for (i, glyph) in glyphs.iter().enumerate().skip(1) {
            if let Some(c) = glyph.unicode {
                by_char.entry(c).or_insert(i);
            }
        }
        Outlines { glyphs, by_char }
    })
}

impl Outlines {
    /// The glyph drawing `c`, if the face has one.
    pub(crate) fn glyph(&self, c: char) -> Option<usize> {
        self.by_char.get(&u32::from(c)).copied()
    }

    /// A glyph's advance in 1/1000 em.
    pub(crate) fn advance(&self, glyph: usize) -> f64 {
        self.glyphs.get(glyph).map_or(0.0, |g| f64::from(g.advance))
    }

    /// A glyph's PostScript name.
    pub(crate) fn name(&self, glyph: usize) -> &'static [u8] {
        self.glyphs.get(glyph).map_or(b".notdef", |g| g.name)
    }

    /// A bare CFF font named `name` of .notdef and `glyphs`, in that order,
    /// each under its own name and with its advance (the /Widths the PDF
    /// gives it), with URW's copyright and the OFL notice; its bounding box
    /// in 1/1000 em.
    pub(crate) fn program(&self, name: &[u8], glyphs: &[usize]) -> Option<(Vec<u8>, [i32; 4])> {
        let mut outlines = vec![(b".notdef".as_slice(), [].as_slice())];
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &i in glyphs {
            let glyph = self.glyphs.get(i)?;
            outlines.push((glyph.name, glyph.commands));
            for command in glyph.commands {
                use crate::builtin_outlines::BuiltinOutlineCommand as C;
                let points: &[(i32, i32)] = match *command {
                    C::MoveTo { x, y } | C::LineTo { x, y } => &[(x, y)],
                    C::CubicTo {
                        x1,
                        y1,
                        x2,
                        y2,
                        x,
                        y,
                    } => &[(x1, y1), (x2, y2), (x, y)],
                    C::Close => &[],
                };
                for &(x, y) in points {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        let bbox = if x0 > x1 { [0; 4] } else { [x0, y0, x1, y1] };
        let mut widths = vec![0u16];
        widths.extend(
            glyphs
                .iter()
                .map(|&i| self.glyphs.get(i).map_or(0, |g| g.advance)),
        );
        let extras = CffExtras {
            widths: &widths,
            copyright: Some(NIMBUS_COPYRIGHT),
            notice: Some(NIMBUS_NOTICE),
        };
        Some((cff_program(name, &outlines, 0.0, &extras)?, bbox))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nimbus_outlines_cover_greek_cyrillic_and_latin_extended() {
        let regular = outlines(false);
        for c in ['Ω', 'Ж', 'ż', 'ő', '€', '→', '≤'] {
            assert!(regular.glyph(c).is_some(), "{c}");
        }
        let (program, bbox) = regular
            .program(b"Test", &[regular.glyph('Ω').unwrap()])
            .unwrap();
        assert_eq!(&program[..4], &[1, 0, 4, 4]);
        assert!(bbox[2] > bbox[0] && bbox[3] > bbox[1]);
        assert!(outlines(true).glyph('Ж').is_some());
    }
}
