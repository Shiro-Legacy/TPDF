// SPDX-License-Identifier: AGPL-3.0-or-later
//! The font programs `scan-compose` embeds (app/ops/scan.rs): a subset of
//! the installed CJK fallback font (TrueType, for a CIDFontType2) and
//! subsets of the URW Nimbus Sans outlines the engine carries (bare CFF,
//! for an embedded Type1C simple font). A child of the view's fonts, so it
//! reuses their sfnt reader and writer, cmap reading, composite glyph
//! flattening and CFF writer rather than a second copy of each.
use super::{
    CJK, CJK_REFUSED, CJK_WANTED, Glyph, MAX_FLATTEN_WORK, Ordering, Points, PoisonError, Sfnt,
    bmp_mapping, cff, glyph_data, glyphs_of, i16_at, own_metric, sfnt, u16_at, ucs4_runs,
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

    /// A font program whose glyph `i` is this font's glyph `glyphs[i]`
    /// (`glyphs[0]` should be 0, .notdef): composite glyphs flattened into
    /// simple ones (so no other glyph joins), each glyph's advance and side
    /// bearing, and the hinting tables (cvt, fpgm, prep) the glyphs'
    /// instructions use. No cmap: a CIDFontType2 with an Identity
    /// CIDToGIDMap selects glyphs by CID. None when the program would pass
    /// the font's own glyph data by more than 1 MiB or the flattening
    /// budget.
    pub(crate) fn subset(&self, glyphs: &[u16]) -> Option<Vec<u8>> {
        let count = u16::try_from(glyphs.len()).ok()?;
        let budget = self.glyf.len().saturating_add(1 << 20);
        let mut data: Vec<u8> = Vec::new();
        let mut offsets: Vec<u8> = Vec::with_capacity(4 * (glyphs.len() + 1));
        let mut horizontal: Vec<u8> = Vec::with_capacity(4 * glyphs.len());
        let (mut points, mut contours, mut widest) = (0u16, 0u16, 0u16);
        let (mut outline, mut work) = (Points::default(), 0usize);
        for &glyph in glyphs {
            let glyph = if glyph < self.glyphs { glyph } else { 0 };
            offsets.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
            let start = data.len();
            let bytes = glyph_data(self.glyf, self.loca, self.long_loca, glyph);
            if bytes.first().is_some_and(|n| n & 0x80 != 0) {
                outline.clear();
                let read =
                    outline.composite(self.glyf, self.loca, self.long_loca, bytes, 4, &mut work);
                if work > MAX_FLATTEN_WORK {
                    return None;
                }
                match read.and_then(|()| outline.write(&mut data)) {
                    Some((n, c)) => (points, contours) = (points.max(n), contours.max(c)),
                    None => data.truncate(start),
                }
            } else {
                data.extend_from_slice(bytes);
            }
            let (advance, bearing) = own_metric(self.hmtx, self.long_metrics, self.last, glyph);
            widest = widest.max(advance);
            horizontal.extend_from_slice(&advance.to_be_bytes());
            horizontal.extend_from_slice(&bearing);
            data.resize(data.len().next_multiple_of(4), 0);
            if data.len() > budget {
                return None;
            }
        }
        offsets.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
        let mut head = self.head.get(..54)?.to_vec();
        head.get_mut(8..12)?.copy_from_slice(&[0; 4]);
        head.get_mut(50..52)?.copy_from_slice(&1u16.to_be_bytes());
        let mut hhea = self.hhea.get(..36)?.to_vec();
        hhea.get_mut(10..12)?.copy_from_slice(&widest.to_be_bytes());
        hhea.get_mut(34..36)?.copy_from_slice(&count.to_be_bytes());
        let mut maxp = self.maxp.to_vec();
        maxp.get_mut(4..6)?.copy_from_slice(&count.to_be_bytes());
        if maxp.len() >= 10 {
            for (at, v) in [(6, points), (8, contours)] {
                let old = u16_at(&maxp, at)?;
                maxp.get_mut(at..at + 2)?
                    .copy_from_slice(&old.max(v).to_be_bytes());
            }
        }
        // post 3.0: no glyph names (they would be by the font's glyph ids).
        let mut post = vec![0u8; 32];
        if let Some(own) = self.font.table(b"post").and_then(|post| post.get(..32)) {
            post.copy_from_slice(own);
        }
        post[..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
        let mut tables: Vec<([u8; 4], &[u8])> = vec![
            (*b"glyf", &data),
            (*b"head", &head),
            (*b"hhea", &hhea),
            (*b"hmtx", &horizontal),
            (*b"loca", &offsets),
            (*b"maxp", &maxp),
            (*b"post", &post),
        ];
        for tag in [b"cvt ", b"fpgm", b"prep"] {
            if let Some(table) = self.font.table(tag) {
                tables.push((*tag, table));
            }
        }
        tables.sort_by_key(|(tag, _)| *tag);
        let mut out = sfnt(*b"\0\x01\0\0", &tables)?;
        checksum_adjustment(&mut out);
        Some(out)
    }
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
    /// each under its own name; its bounding box in 1/1000 em.
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
        Some((cff(name, &outlines, 0.0)?, bbox))
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
