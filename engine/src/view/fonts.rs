// SPDX-License-Identifier: AGPL-3.0-or-later
//! The fonts the view gives hayro for text whose font program is not
//! embedded. MuPDF draws the base 14 fonts with URW's; the faces whose URW
//! outlines the engine carries (`builtin_outlines`: Helvetica, Helvetica-Bold,
//! Helvetica-BoldOblique, Times-Roman, and Helvetica-Oblique as Helvetica
//! slanted 12°, which URW's is) are built into OpenType fonts with CFF
//! outlines on first use, so their Cyrillic, Greek and other non-Latin
//! glyphs draw. The other faces keep hayro's Foxit fonts, Latin only.
//!
//! CID fonts of a Chinese, Japanese or Korean collection draw with a font the
//! host installs (`set_cjk_font`, MuPDF's Droid Sans Fallback), loaded on
//! demand. Until one is installed they get a font without glyphs, so they draw
//! nothing (hayro would otherwise read their two-byte codes as Helvetica text)
//! while their text is still extracted, and `cjk_font_wanted` turns true.
use crate::builtin_metrics::BuiltinFace;
use crate::builtin_outlines::{BuiltinOutline, BuiltinOutlineCommand};
use hayro_interpret::font::{FallbackFontQuery, FontData, FontQuery, StandardFont};
use hayro_interpret::hayro_cmap::CidFamily;
use hayro_interpret::{FontResolverFn, InterpreterSettings};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// The largest CJK font `set_cjk_font` takes (Droid Sans Fallback is 3.3 MiB).
pub const MAX_CJK_FONT_BYTES: usize = 32 << 20;

/// Interpreter settings with the view's fonts, for drawing (`paint`: a
/// CJK font missing then turns `cjk_font_wanted` on) or text extraction.
pub(crate) fn settings(render_annotations: bool, paint: bool) -> InterpreterSettings {
    InterpreterSettings {
        font_resolver: resolver(paint),
        render_annotations,
        ..InterpreterSettings::default()
    }
}

fn resolver(paint: bool) -> FontResolverFn {
    static RESOLVERS: [OnceLock<FontResolverFn>; 2] = [OnceLock::new(), OnceLock::new()];
    RESOLVERS[usize::from(paint)]
        .get_or_init(|| {
            Arc::new(move |query: &FontQuery| match query {
                FontQuery::Standard(font) => Some(standard(*font)),
                FontQuery::Fallback(query) => Some(fallback(query, paint)),
            })
        })
        .clone()
}

/// A base 14 font: URW where the engine has the outlines, else hayro's own.
fn standard(font: StandardFont) -> (FontData, u32) {
    let face = match font {
        StandardFont::Helvetica => Some(Face::Helvetica),
        StandardFont::HelveticaBold => Some(Face::HelveticaBold),
        StandardFont::HelveticaOblique => Some(Face::HelveticaOblique),
        StandardFont::HelveticaBoldOblique => Some(Face::HelveticaBoldOblique),
        StandardFont::TimesRoman => Some(Face::TimesRoman),
        _ => None,
    };
    match face.and_then(Face::program) {
        Some(program) => (program, 0),
        None => font.get_font_data(),
    }
}

/// A CID font without its program: the installed CJK font for a CJK
/// collection, else the base 14 font hayro picks from the descriptor.
fn fallback(query: &FallbackFontQuery, paint: bool) -> (FontData, u32) {
    let cjk = query.character_collection.as_ref().is_some_and(|c| {
        matches!(
            c.family,
            CidFamily::AdobeGB1
                | CidFamily::AdobeCNS1
                | CidFamily::AdobeJapan1
                | CidFamily::AdobeKorea1
        )
    });
    if !cjk {
        return standard(query.pick_standard_font());
    }
    let installed = CJK.lock().unwrap_or_else(PoisonError::into_inner).clone();
    match installed {
        Some(font) => (font, 0),
        None => {
            if paint && !CJK_REFUSED.load(Ordering::Relaxed) {
                CJK_WANTED.store(true, Ordering::Relaxed);
            }
            (empty(), 0)
        }
    }
}

static CJK: Mutex<Option<FontData>> = Mutex::new(None);
static CJK_WANTED: AtomicBool = AtomicBool::new(false);
/// The last `set_cjk_font` refused its font: renders no longer ask for one.
static CJK_REFUSED: AtomicBool = AtomicBool::new(false);

/// Why `set_cjk_font` refused a font.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontError {
    TooLarge,
    /// Not a TrueType or OpenType font with the tables a CJK font needs.
    NotAFont,
}

impl std::fmt::Display for FontError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "the font is larger than the limit",
            Self::NotAFont => "not a TrueType or OpenType font with a cmap",
        })
    }
}

impl std::error::Error for FontError {}

/// Installs the font every view draws CJK CID fonts without a program in,
/// for the whole process (views made before draw with it from their next
/// render): a TrueType or OpenType font with a Unicode cmap, such as
/// MuPDF's Droid Sans Fallback. A TrueType font gets URW Helvetica's glyphs
/// for the characters it lacks, and an empty glyph for every other
/// character (`with_total_cmap`): hayro draws a character a font's cmap
/// lacks as the glyph numbered like its CID, a wrong one.
///
/// Either way `cjk_font_wanted` turns false; after a refusal renders no
/// longer turn it on (until a font is installed), so a host that renders
/// again on the cue cannot loop on a font it cannot install.
pub fn set_cjk_font(bytes: Vec<u8>) -> Result<(), FontError> {
    let installed = install_cjk_font(bytes);
    CJK_REFUSED.store(installed.is_err(), Ordering::Relaxed);
    CJK_WANTED.store(false, Ordering::Relaxed);
    installed
}

fn install_cjk_font(bytes: Vec<u8>) -> Result<(), FontError> {
    if bytes.len() > MAX_CJK_FONT_BYTES {
        return Err(FontError::TooLarge);
    }
    let font = Sfnt::read(&bytes).ok_or(FontError::NotAFont)?;
    if [b"cmap", b"head", b"maxp"]
        .iter()
        .any(|tag| font.table(tag).is_none())
    {
        return Err(FontError::NotAFont);
    }
    let font = with_total_cmap(&font).unwrap_or(bytes);
    *CJK.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(font));
    Ok(())
}

/// Whether a CJK font was installed.
pub fn has_cjk_font() -> bool {
    CJK.lock().unwrap_or_else(PoisonError::into_inner).is_some()
}

/// Whether some text since the last install attempt needed the CJK font
/// and drew nothing for lack of it: the host's cue to install one and,
/// when `set_cjk_font` takes it, draw again.
pub fn cjk_font_wanted() -> bool {
    CJK_WANTED.load(Ordering::Relaxed)
}

/// An sfnt's tables (of a collection, the first font's).
struct Sfnt<'a> {
    version: [u8; 4],
    tables: Vec<([u8; 4], &'a [u8])>,
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

impl<'a> Sfnt<'a> {
    fn read(bytes: &'a [u8]) -> Option<Self> {
        let start = match bytes.get(..4)? {
            b"ttcf" => usize::try_from(u32_at(bytes, 12)?).ok()?,
            _ => 0,
        };
        let version: [u8; 4] = bytes.get(start..start.checked_add(4)?)?.try_into().ok()?;
        if !matches!(&version, b"\0\x01\0\0" | b"OTTO" | b"true") {
            return None;
        }
        let count = u16_at(bytes, start.checked_add(4)?)?;
        let mut tables = Vec::with_capacity(usize::from(count));
        for i in 0..usize::from(count) {
            let record = start.checked_add(12 + 16 * i)?;
            let tag: [u8; 4] = bytes.get(record..record.checked_add(4)?)?.try_into().ok()?;
            let offset = usize::try_from(u32_at(bytes, record.checked_add(8)?)?).ok()?;
            let len = usize::try_from(u32_at(bytes, record.checked_add(12)?)?).ok()?;
            tables.push((tag, bytes.get(offset..offset.checked_add(len)?)?));
        }
        Some(Self { version, tables })
    }

    fn table(&self, tag: &[u8; 4]) -> Option<&'a [u8]> {
        (self.tables.iter()).find_map(|(t, data)| (t == tag).then_some(*data))
    }
}

/// `tables` (sorted by tag) as one sfnt.
fn sfnt(version: [u8; 4], tables: &[([u8; 4], &[u8])]) -> Option<Vec<u8>> {
    let count = u16::try_from(tables.len()).ok()?;
    let power = if count == 0 {
        0
    } else {
        1u16 << (15 - count.leading_zeros())
    };
    let mut out = Vec::new();
    out.extend_from_slice(&version);
    let range = power.wrapping_mul(16);
    for v in [
        count,
        range,
        power.trailing_zeros() as u16,
        (count - power).wrapping_mul(16),
    ] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in tables {
        let sum = (data.chunks(4)).fold(0u32, |sum, chunk| {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            sum.wrapping_add(u32::from_be_bytes(word))
        });
        out.extend_from_slice(tag);
        out.extend_from_slice(&sum.to_be_bytes());
        out.extend_from_slice(&u32::try_from(offset).ok()?.to_be_bytes());
        out.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
        offset = offset.checked_add(data.len().next_multiple_of(4))?;
    }
    out.try_reserve(offset.saturating_sub(out.len())).ok()?;
    for (_, data) in tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    Some(out)
}

/// The Unicode BMP mapping of a cmap: its UCS-4 format 12 subtable, else
/// its BMP format 4 one; glyph 0 where it maps nothing. None past 2^20
/// mapped characters (overlapping ranges).
fn bmp_mapping(cmap: &[u8]) -> Option<Vec<u16>> {
    let mut work = 0usize;
    let mut charge = |n: usize| {
        work = work.saturating_add(n);
        (work <= 1 << 20).then_some(())
    };
    let count = usize::from(u16_at(cmap, 2)?);
    let mut best: Option<(u8, &[u8])> = None;
    for i in 0..count {
        let (platform, encoding) = (u16_at(cmap, 4 + 8 * i)?, u16_at(cmap, 6 + 8 * i)?);
        let offset = usize::try_from(u32_at(cmap, 8 + 8 * i)?).ok()?;
        let sub = cmap.get(offset..)?;
        let rank = match (platform, encoding, u16_at(sub, 0)?) {
            (3, 10, 12) | (0, 4, 12) => 2,
            (3, 1, 4) | (0, 0..=3, 4) => 1,
            _ => continue,
        };
        if best.is_none_or(|(r, _)| rank > r) {
            best = Some((rank, sub));
        }
    }
    let (rank, sub) = best?;
    let mut gids = vec![0u16; 0x1_0000];
    if rank == 2 {
        let groups = usize::try_from(u32_at(sub, 12)?).ok()?;
        for g in 0..groups.min(sub.len() / 12) {
            let at = 16 + 12 * g;
            let (first, last) = (u32_at(sub, at)?, u32_at(sub, at + 4)?);
            let gid = u32_at(sub, at + 8)?;
            charge((last.min(0xFFFF).saturating_sub(first) as usize).saturating_add(1))?;
            for code in first..=last.min(0xFFFF) {
                let glyph = gid.checked_add(code - first)?;
                gids[code as usize] = u16::try_from(glyph).unwrap_or(0);
            }
        }
    } else {
        let segments = usize::from(u16_at(sub, 6)? / 2);
        let (ends, starts) = (14, 16 + 2 * segments);
        let (deltas, ranges) = (starts + 2 * segments, starts + 4 * segments);
        for s in 0..segments {
            let (first, last) = (u16_at(sub, starts + 2 * s)?, u16_at(sub, ends + 2 * s)?);
            let (delta, range) = (u16_at(sub, deltas + 2 * s)?, u16_at(sub, ranges + 2 * s)?);
            charge(usize::from(last.saturating_sub(first)) + 1)?;
            for code in first..=last {
                let glyph = if range == 0 {
                    code.wrapping_add(delta)
                } else {
                    let at = ranges + 2 * s + usize::from(range) + 2 * usize::from(code - first);
                    match u16_at(sub, at) {
                        Some(0) | None => 0,
                        Some(glyph) => glyph.wrapping_add(delta),
                    }
                };
                gids[usize::from(code)] = glyph;
            }
        }
    }
    Some(gids)
}

/// A TrueType font with more glyphs and a cmap with one format 12
/// subtable: the font's own BMP mapping; for each character it lacks that
/// URW Helvetica has, that glyph, appended with URW's advance (MuPDF draws a
/// CJK font's missing Latin, digits and symbols from its other fallback
/// fonts); for every other BMP character but the surrogates and U+FFFF, an
/// appended empty glyph; and past the BMP the font's own characters and
/// appended empty glyphs for the rest (`past_bmp_groups`). None for a font
/// that is not TrueType or has no room for glyphs.
fn with_total_cmap(font: &Sfnt<'_>) -> Option<Vec<u8>> {
    let (head, maxp, loca, glyf, hmtx) = (
        font.table(b"head")?,
        font.table(b"maxp")?,
        font.table(b"loca")?,
        font.table(b"glyf")?,
        font.table(b"hmtx")?,
    );
    let glyphs = u16_at(maxp, 4)?;
    let units = f64::from(u16_at(head, 18)?.clamp(16, 16_384)) / 1000.0;
    let long = u16_at(head, 50)? == 1;
    let offset = |i: usize| -> Option<usize> {
        if long {
            usize::try_from(u32_at(loca, 4 * i)?).ok()
        } else {
            Some(2 * usize::from(u16_at(loca, 2 * i)?))
        }
    };
    let mut gids = bmp_mapping(font.table(b"cmap")?)?;
    for gid in gids.iter_mut().filter(|gid| **gid >= glyphs) {
        *gid = 0;
    }
    // Glyph data and offsets, rewritten in the long format.
    let mut data = glyf.get(..offset(usize::from(glyphs))?)?.to_vec();
    let mut offsets = Vec::with_capacity(4 * (usize::from(glyphs) + 700));
    for i in 0..=usize::from(glyphs) {
        offsets.extend_from_slice(&u32::try_from(offset(i)?).ok()?.to_be_bytes());
    }
    let (mut count, mut points, mut contours) = (glyphs, 0u16, 0u16);
    // Left side bearings of the appended glyphs (the hinter places a glyph
    // by its bearing) and their advances, URW's.
    let mut bearings: Vec<i16> = Vec::new();
    let mut advances: Vec<u16> = Vec::new();
    for glyph in glyphs_of(BuiltinFace::Helvetica).iter().skip(1) {
        let Some(code) = glyph.unicode.and_then(|u| usize::try_from(u).ok()) else {
            continue;
        };
        if gids.get(code).is_none_or(|&gid| gid != 0) || count >= u16::MAX - 2 {
            continue;
        }
        let Some((bytes, n, c)) = truetype_glyph(glyph.commands, units) else {
            continue;
        };
        bearings.push(
            bytes
                .get(2..4)
                .map_or(0, |b| i16::from_be_bytes([b[0], b[1]])),
        );
        advances.push((f64::from(glyph.advance) * units).round() as u16);
        data.extend_from_slice(&bytes);
        data.resize(data.len().next_multiple_of(4), 0);
        offsets.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
        gids[code] = count;
        count += 1;
        (points, contours) = (points.max(n), contours.max(c));
    }
    let empty = count;
    if empty >= u16::MAX - 1 {
        return None;
    }
    offsets.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
    count += 1;
    for (code, gid) in gids.iter_mut().enumerate() {
        if *gid == 0 {
            *gid = if (0xD800..0xE000).contains(&code) || code == 0xFFFF {
                0
            } else {
                empty
            };
        }
    }
    // Runs of characters mapped to consecutive glyphs; the empty glyph's
    // characters one group each.
    let mut groups: Vec<(u32, u32, u16)> = Vec::new();
    for (code, &gid) in gids.iter().enumerate() {
        let code = code as u32;
        if gid == 0 {
            continue;
        }
        if let Some((first, last, first_gid)) = groups.last_mut()
            && *last + 1 == code
            && gid != empty
            && u32::from(*first_gid) + (code - *first) == u32::from(gid)
        {
            *last = code;
            continue;
        }
        groups.push((code, code, gid));
    }
    // Past the BMP: the font's own characters, and every other one mapped,
    // in runs, onto a block of appended empty glyphs (a format 12 group maps
    // a run to consecutive glyphs). hayro draws a character the cmap lacks
    // as the glyph numbered like its CID, a wrong one.
    let block = (u16::MAX - count).min(PAST_BMP_BLOCK);
    let past = (block >= PAST_BMP_BLOCK_MIN)
        .then(|| past_bmp_groups(font.table(b"cmap")?, glyphs, count, block))
        .flatten();
    if let Some(past) = past {
        groups.extend(past);
        for _ in 0..block {
            offsets.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
        }
        count += block;
    }
    let mut cmap = Vec::with_capacity(28 + 12 * groups.len());
    for v in [0u16, 1, 3, 10] {
        cmap.extend_from_slice(&v.to_be_bytes());
    }
    cmap.extend_from_slice(&12u32.to_be_bytes());
    cmap.extend_from_slice(&12u16.to_be_bytes());
    cmap.extend_from_slice(&0u16.to_be_bytes());
    let length = u32::try_from(16 + 12 * groups.len()).ok()?;
    cmap.extend_from_slice(&length.to_be_bytes());
    cmap.extend_from_slice(&0u32.to_be_bytes());
    cmap.extend_from_slice(&u32::try_from(groups.len()).ok()?.to_be_bytes());
    for (first, last, gid) in &groups {
        for v in [*first, *last, u32::from(*gid)] {
            cmap.extend_from_slice(&v.to_be_bytes());
        }
    }
    let mut head = head.to_vec();
    head.get_mut(50..52)?.copy_from_slice(&1u16.to_be_bytes());
    let mut maxp = maxp.to_vec();
    maxp.get_mut(4..6)?.copy_from_slice(&count.to_be_bytes());
    if maxp.len() >= 10 {
        // Version 1.0: maxPoints and maxContours cover the new glyphs.
        let at_least = |maxp: &mut Vec<u8>, at: usize, v: u16| {
            let old = u16_at(maxp, at).unwrap_or(0);
            maxp[at..at + 2].copy_from_slice(&old.max(v).to_be_bytes());
        };
        at_least(&mut maxp, 6, points);
        at_least(&mut maxp, 8, contours);
    }
    bearings.push(0);
    // Glyphs past numOfLongVerMetrics have a side bearing only; the table is
    // cut to its glyphs first.
    let metrics = |table: &[u8], header: Option<&[u8]>, bearings: &[i16]| {
        let long = header
            .and_then(|h| u16_at(h, 34))
            .unwrap_or(glyphs)
            .min(glyphs);
        let len = 4 * usize::from(long) + 2 * usize::from(glyphs - long);
        let mut out = table.get(..len)?.to_vec();
        out.extend(bearings.iter().flat_map(|b| b.to_be_bytes()));
        Some(out)
    };
    // Every glyph gets a long horizontal metric: past numberOfHMetrics an
    // appended glyph would take the last advance (a full em in Droid Sans
    // Fallback), and hayro stretches a CID font's fallback glyph to its /W
    // width by its advance, so URW's Latin drew squeezed. The empty glyph
    // keeps the last advance.
    let hhea = font.table(b"hhea");
    let long = hhea
        .and_then(|h| u16_at(h, 34))
        .unwrap_or(glyphs)
        .min(glyphs);
    let last = match long.checked_sub(1) {
        Some(i) => u16_at(hmtx, 4 * usize::from(i))?,
        None => 0,
    };
    let mut horizontal = Vec::with_capacity(4 * usize::from(count));
    for i in 0..usize::from(glyphs) {
        let (advance, bearing) = if i < usize::from(long) {
            (u16_at(hmtx, 4 * i)?, u16_at(hmtx, 4 * i + 2)?)
        } else {
            let at = 4 * usize::from(long) + 2 * (i - usize::from(long));
            (last, u16_at(hmtx, at)?)
        };
        horizontal.extend_from_slice(&advance.to_be_bytes());
        horizontal.extend_from_slice(&bearing.to_be_bytes());
    }
    // The empty glyph and the block past it keep the last advance.
    bearings.resize(usize::from(count - glyphs), 0);
    advances.resize(bearings.len(), last);
    for (advance, bearing) in advances.iter().zip(&bearings) {
        horizontal.extend_from_slice(&advance.to_be_bytes());
        horizontal.extend_from_slice(&bearing.to_be_bytes());
    }
    let hhea = hhea.map(|h| {
        let mut h = h.to_vec();
        let widest = advances
            .iter()
            .copied()
            .fold(u16_at(&h, 10).unwrap_or(0), u16::max);
        if let Some(field) = h.get_mut(10..12) {
            field.copy_from_slice(&widest.to_be_bytes());
        }
        if let Some(field) = h.get_mut(34..36) {
            field.copy_from_slice(&count.to_be_bytes());
        }
        h
    });
    let vmtx = font
        .table(b"vmtx")
        .and_then(|vmtx| metrics(vmtx, font.table(b"vhea"), &vec![0; bearings.len()]));
    let mut tables: Vec<([u8; 4], &[u8])> = Vec::new();
    for (tag, table) in &font.tables {
        let data: &[u8] = match tag {
            b"cmap" => &cmap,
            b"glyf" => &data,
            b"head" => &head,
            b"loca" => &offsets,
            b"maxp" => &maxp,
            b"hhea" => hhea.as_deref().unwrap_or(table),
            b"hmtx" => &horizontal,
            b"vmtx" => vmtx.as_deref().unwrap_or(table),
            _ => table,
        };
        tables.push((*tag, data));
    }
    tables.sort_by_key(|(tag, _)| *tag);
    sfnt(font.version, &tables)
}

/// Empty glyphs appended for the characters past the BMP a CJK font lacks:
/// at most this many (runs of up to this many characters share them), and
/// none when fewer than the minimum fit under 65,535 glyphs.
const PAST_BMP_BLOCK: u16 = 4096;
const PAST_BMP_BLOCK_MIN: u16 = 256;

/// Format 12 groups for U+10000 to U+10FFFF: the runs the font's UCS-4
/// subtable maps there (to glyphs below `glyphs`; overlapping runs are
/// dropped), and every other character in runs of at most `block` mapped
/// onto the glyphs from `empty`. None past 2^16 groups in the font.
fn past_bmp_groups(
    cmap: &[u8],
    glyphs: u16,
    empty: u16,
    block: u16,
) -> Option<Vec<(u32, u32, u16)>> {
    let count = usize::from(u16_at(cmap, 2)?);
    let mut own: Vec<(u32, u32, u16)> = Vec::new();
    for i in 0..count {
        let (platform, encoding) = (u16_at(cmap, 4 + 8 * i)?, u16_at(cmap, 6 + 8 * i)?);
        let offset = usize::try_from(u32_at(cmap, 8 + 8 * i)?).ok()?;
        let sub = cmap.get(offset..)?;
        if !matches!(
            (platform, encoding, u16_at(sub, 0)?),
            (3, 10, 12) | (0, 4, 12)
        ) {
            continue;
        }
        let groups = usize::try_from(u32_at(sub, 12)?).ok()?;
        if groups > 1 << 16 {
            return None;
        }
        for g in 0..groups.min(sub.len() / 12) {
            let at = 16 + 12 * g;
            let (start, end, gid) = (u32_at(sub, at)?, u32_at(sub, at + 4)?, u32_at(sub, at + 8)?);
            let (first, last) = (start.max(0x1_0000), end.min(0x10_FFFF));
            // The run from where it enters the planes, cut to the font's glyphs.
            let Some(glyph) = gid.checked_add(first - start) else {
                continue;
            };
            let room = u32::from(glyphs).saturating_sub(glyph);
            if first > last || room == 0 {
                continue;
            }
            own.push((
                first,
                last.min(first + (room - 1)),
                u16::try_from(glyph).ok()?,
            ));
        }
        break;
    }
    own.sort_unstable();
    let mut out = Vec::new();
    let mut next = 0x1_0000u32;
    let fill = |out: &mut Vec<(u32, u32, u16)>, from: u32, to: u32| {
        let mut at = from;
        while at <= to {
            let end = to.min(at + u32::from(block) - 1);
            out.push((at, end, empty));
            at = end + 1;
        }
    };
    for (first, last, gid) in own {
        if first < next {
            continue;
        }
        fill(&mut out, next, first - 1);
        out.push((first, last, gid));
        next = last + 1;
    }
    fill(&mut out, next, 0x10_FFFF);
    Some(out)
}

/// A simple TrueType glyph of `commands` (1/1000 em) scaled by `units`,
/// each cubic as two quadratics; with its point and contour counts.
fn truetype_glyph(commands: &[BuiltinOutlineCommand], units: f64) -> Option<(Vec<u8>, u16, u16)> {
    type P = (f64, f64);
    let scale = |x: i32, y: i32| (f64::from(x) * units, f64::from(y) * units);
    let mut contours: Vec<Vec<(P, bool)>> = Vec::new();
    let mut at = (0.0, 0.0);
    for command in commands {
        match *command {
            BuiltinOutlineCommand::MoveTo { x, y } => {
                at = scale(x, y);
                contours.push(vec![(at, true)]);
            }
            BuiltinOutlineCommand::LineTo { x, y } => {
                at = scale(x, y);
                contours.last_mut()?.push((at, true));
            }
            BuiltinOutlineCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                let (p0, p1, p2, p3) = (at, scale(x1, y1), scale(x2, y2), scale(x, y));
                let mid = |a: P, b: P| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                // Split at t = 1/2 and approximate each half by one quadratic.
                let (a, b, c) = (mid(p0, p1), mid(p1, p2), mid(p2, p3));
                let (d, e) = (mid(a, b), mid(b, c));
                let m = mid(d, e);
                let control = |s: P, c1: P, c2: P, t: P| {
                    (
                        (3.0 * (c1.0 + c2.0) - s.0 - t.0) / 4.0,
                        (3.0 * (c1.1 + c2.1) - s.1 - t.1) / 4.0,
                    )
                };
                let contour = contours.last_mut()?;
                contour.push((control(p0, a, d, m), false));
                contour.push((m, true));
                contour.push((control(m, e, c, p3), false));
                contour.push((p3, true));
                at = p3;
            }
            BuiltinOutlineCommand::Close => {}
        }
    }
    let round = |v: f64| v.round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
    let mut points: Vec<((i16, i16), bool)> = Vec::new();
    let mut ends: Vec<u16> = Vec::new();
    for mut contour in contours {
        // TrueType closes a contour itself.
        if contour.len() > 1 && contour.first().map(|p| p.0) == contour.last().map(|p| p.0) {
            contour.pop();
        }
        if contour.len() < 2 {
            continue;
        }
        points.extend(
            contour
                .iter()
                .map(|&((x, y), on)| ((round(x), round(y)), on)),
        );
        ends.push(u16::try_from(points.len() - 1).ok()?);
    }
    let mut out = Vec::new();
    if points.is_empty() {
        return Some((out, 0, 0));
    }
    let xs = points.iter().map(|p| p.0.0);
    let ys = points.iter().map(|p| p.0.1);
    let (x0, x1) = (xs.clone().min()?, xs.max()?);
    let (y0, y1) = (ys.clone().min()?, ys.max()?);
    for v in [i16::try_from(ends.len()).ok()?, x0, y0, x1, y1] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    for end in &ends {
        out.extend_from_slice(&end.to_be_bytes());
    }
    out.extend_from_slice(&0u16.to_be_bytes());
    // Flags: on curve or not; coordinates as signed 16-bit deltas.
    out.extend(points.iter().map(|&(_, on)| u8::from(on)));
    for axis in 0..2 {
        let mut last = 0i16;
        for &((x, y), _) in &points {
            let v = if axis == 0 { x } else { y };
            out.extend_from_slice(&v.wrapping_sub(last).to_be_bytes());
            last = v;
        }
    }
    let n = u16::try_from(points.len()).ok()?;
    Some((out, n, u16::try_from(ends.len()).ok()?))
}

#[path = "fonts_unicode.rs"]
mod unicode_table;

/// The faces built from the engine's outlines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Face {
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    HelveticaBoldOblique,
    TimesRoman,
}

/// tan 12°: URW's Nimbus Sans Italic is its regular face slanted by 12°.
const OBLIQUE: f64 = 0.212_556_561_670_022_13;

impl Face {
    /// The outlines and the slant applied to them.
    fn builtin(self) -> (BuiltinFace, f64) {
        match self {
            Self::Helvetica => (BuiltinFace::Helvetica, 0.0),
            Self::HelveticaBold => (BuiltinFace::HelveticaBold, 0.0),
            Self::HelveticaOblique => (BuiltinFace::Helvetica, OBLIQUE),
            Self::HelveticaBoldOblique => (BuiltinFace::HelveticaBoldOblique, 0.0),
            Self::TimesRoman => (BuiltinFace::TimesRoman, 0.0),
        }
    }

    fn slot(self) -> &'static OnceLock<Option<FontData>> {
        static SLOTS: [OnceLock<Option<FontData>>; 5] = [
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
        ];
        &SLOTS[self as usize]
    }

    /// The face as an OpenType font with CFF outlines, built once per
    /// process.
    fn program(self) -> Option<FontData> {
        self.slot()
            .get_or_init(|| {
                let (face, slant) = self.builtin();
                let bytes = otf(face.pdf_name(), &glyphs_of(face), slant)?;
                Some(Arc::new(bytes) as FontData)
            })
            .clone()
    }
}

/// One glyph of a font the view builds.
struct Glyph {
    name: &'static [u8],
    commands: &'static [BuiltinOutlineCommand],
    /// Advance in 1/1000 em.
    advance: u16,
    unicode: Option<u32>,
}

/// A face's glyphs, .notdef first (glyph 0).
fn glyphs_of(face: BuiltinFace) -> Vec<Glyph> {
    let mut glyphs = vec![Glyph {
        name: b".notdef",
        commands: &[],
        advance: 0,
        unicode: None,
    }];
    glyphs.extend(
        (BuiltinOutline::glyphs(face))
            .filter(|glyph| glyph.name() != b".notdef")
            .map(|glyph| Glyph {
                name: glyph.name(),
                commands: glyph.commands(),
                advance: face
                    .lookup_glyph(glyph.name())
                    .map_or(0, |advance| advance.value().clamp(0.0, 65_535.0) as u16),
                unicode: unicode(glyph.name()),
            }),
    );
    glyphs
}

/// The Unicode scalar a URW glyph name stands for.
fn unicode(name: &[u8]) -> Option<u32> {
    let table = &unicode_table::UNICODE;
    let at = table.binary_search_by(|(key, _)| key.cmp(&name)).ok()?;
    Some(table[at].1)
}

/// A CFF program with no glyph but .notdef: hayro reads the CID font's
/// codes and draws nothing.
fn empty() -> FontData {
    static EMPTY: OnceLock<FontData> = OnceLock::new();
    EMPTY
        .get_or_init(|| {
            let bytes = cff(b"Empty", &[(b".notdef".as_slice(), [].as_slice())], 0.0);
            Arc::new(bytes.unwrap_or_default()) as FontData
        })
        .clone()
}

/// The first string ID of a CFF's own String INDEX.
const FIRST_CUSTOM_SID: usize = 391;

/// A bare CFF font (Adobe TN 5176) named `name` whose glyph `i` is
/// `glyphs[i]`: its name (every name in the String INDEX) and its outline
/// as Type 2 charstrings in units of 1/1000 em, with no widths or hints.
/// None when there are more glyphs than a CFF holds.
fn cff(name: &[u8], glyphs: &[(&[u8], &[BuiltinOutlineCommand])], slant: f64) -> Option<Vec<u8>> {
    let count = glyphs.len();
    if count == 0 || count > usize::from(u16::MAX) || FIRST_CUSTOM_SID + count > 65_000 {
        return None;
    }
    let charstrings: Vec<Vec<u8>> = glyphs
        .iter()
        .map(|(_, commands)| charstring(commands, slant))
        .collect();
    let names: Vec<&[u8]> = glyphs.iter().skip(1).map(|(name, _)| *name).collect();
    let name_index = index(&[name]);
    let string_index = index(&names);
    let global_subrs = index(&[]);
    let mut charset = vec![0u8];
    for i in 0..names.len() {
        let sid = u16::try_from(FIRST_CUSTOM_SID + i).ok()?;
        charset.extend_from_slice(&sid.to_be_bytes());
    }
    let refs: Vec<&[u8]> = charstrings.iter().map(Vec::as_slice).collect();
    let charstring_index = index(&refs);
    // defaultWidthX 0, nominalWidthX 0.
    let private = [139, 20, 139, 21];
    // The Top DICT's offsets are 5-byte integers, so its size is fixed.
    let top_len = 3 * 6 + 5;
    let top_index_len = 2 + 1 + 2 + top_len;
    let charset_at = 4 + name_index.len() + top_index_len + string_index.len() + global_subrs.len();
    let charstrings_at = charset_at + charset.len();
    let private_at = charstrings_at + charstring_index.len();
    let mut top = Vec::with_capacity(top_len);
    for (value, op) in [(charset_at, 15u8), (charstrings_at, 17)] {
        dict_int(&mut top, value)?;
        top.push(op);
    }
    dict_int(&mut top, private.len())?;
    dict_int(&mut top, private_at)?;
    top.push(18);
    let top_index = index(&[&top]);
    let mut out = Vec::with_capacity(private_at + private.len());
    out.extend_from_slice(&[1, 0, 4, 4]);
    for part in [
        name_index.as_slice(),
        &top_index,
        &string_index,
        &global_subrs,
        &charset,
        &charstring_index,
        &private,
    ] {
        out.extend_from_slice(part);
    }
    (out.len() == private_at + private.len()).then_some(out)
}

/// An OpenType font (CFF outlines) of `glyphs`, glyph 0 being .notdef:
/// the CFF program, advances (hmtx), names (post 2.0) and a Unicode cmap,
/// through which hayro finds a glyph by any name the Adobe Glyph List gives
/// its character (`Acyrillic` as well as the URW `afii10017`).
fn otf(name: &[u8], glyphs: &[Glyph], slant: f64) -> Option<Vec<u8>> {
    let count = u16::try_from(glyphs.len()).ok()?;
    let outlines: Vec<(&[u8], &[BuiltinOutlineCommand])> =
        glyphs.iter().map(|g| (g.name, g.commands)).collect();
    let cff = cff(name, &outlines, slant)?;
    let (mut x0, mut y0, mut x1, mut y1) = (0i16, 0i16, 0i16, 0i16);
    for command in glyphs.iter().flat_map(|g| g.commands) {
        let points: &[(i32, i32)] = match *command {
            BuiltinOutlineCommand::MoveTo { x, y } | BuiltinOutlineCommand::LineTo { x, y } => {
                &[(x, y)]
            }
            BuiltinOutlineCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => &[(x1, y1), (x2, y2), (x, y)],
            BuiltinOutlineCommand::Close => &[],
        };
        for &(x, y) in points {
            let x = x.saturating_add((f64::from(y) * slant).round() as i32);
            let clamp = |v: i32| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            (x0, y0) = (x0.min(clamp(x)), y0.min(clamp(y)));
            (x1, y1) = (x1.max(clamp(x)), y1.max(clamp(y)));
        }
    }
    let be16 = |out: &mut Vec<u8>, v: u16| out.extend_from_slice(&v.to_be_bytes());
    let be32 = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_be_bytes());
    let mut head = Vec::with_capacity(54);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0x0001_0000);
    be32(&mut head, 0);
    be32(&mut head, 0x5F0F_3CF5);
    be16(&mut head, 0);
    be16(&mut head, 1000);
    head.extend_from_slice(&[0; 16]);
    for v in [x0, y0, x1, y1] {
        head.extend_from_slice(&v.to_be_bytes());
    }
    for v in [0u16, 8, 2, 0, 0] {
        be16(&mut head, v);
    }
    let max_advance = glyphs.iter().map(|g| g.advance).max().unwrap_or(0);
    let mut hhea = Vec::with_capacity(36);
    be32(&mut hhea, 0x0001_0000);
    for v in [y1, y0, 0] {
        hhea.extend_from_slice(&v.to_be_bytes());
    }
    be16(&mut hhea, max_advance);
    for v in [x0, 0, x1] {
        hhea.extend_from_slice(&v.to_be_bytes());
    }
    // Caret rise and run, caret offset, four reserved, metric data format.
    for v in [1u16, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut hhea, v);
    }
    be16(&mut hhea, count);
    let mut hmtx = Vec::with_capacity(4 * glyphs.len());
    for glyph in glyphs {
        be16(&mut hmtx, glyph.advance);
        be16(&mut hmtx, 0);
    }
    let mut maxp = Vec::with_capacity(6);
    be32(&mut maxp, 0x0000_5000);
    be16(&mut maxp, count);
    let mut post = Vec::new();
    be32(&mut post, 0x0002_0000);
    post.extend_from_slice(&[0; 28]);
    be16(&mut post, count);
    for i in 0..count {
        // Index 0 is the Macintosh .notdef; the others are this table's names.
        be16(&mut post, if i == 0 { 0 } else { 257 + i });
    }
    for glyph in glyphs.iter().skip(1) {
        let len = u8::try_from(glyph.name.len()).ok()?;
        post.push(len);
        post.extend_from_slice(glyph.name);
    }
    let mut pairs: Vec<(u16, u16)> = (glyphs.iter().enumerate())
        .filter_map(|(gid, g)| Some((u16::try_from(g.unicode?).ok()?, gid as u16)))
        .filter(|&(code, _)| code != 0xFFFF)
        .collect();
    pairs.sort_unstable();
    pairs.dedup_by_key(|pair| pair.0);
    let cmap = cmap4(&pairs);
    let tables: [([u8; 4], &[u8]); 7] = [
        (*b"CFF ", &cff),
        (*b"cmap", &cmap),
        (*b"head", &head),
        (*b"hhea", &hhea),
        (*b"hmtx", &hmtx),
        (*b"maxp", &maxp),
        (*b"post", &post),
    ];
    sfnt(*b"OTTO", &tables)
}

/// A cmap with one format 4 subtable (Windows, Unicode BMP) mapping each
/// code of `pairs` (sorted, distinct, below U+FFFF) to its glyph: one
/// segment per code, then the final 0xFFFF segment.
fn cmap4(pairs: &[(u16, u16)]) -> Vec<u8> {
    let segments = pairs.len() + 1;
    let mut sub = Vec::with_capacity(16 + 8 * segments);
    let be16 = |out: &mut Vec<u8>, v: u16| out.extend_from_slice(&v.to_be_bytes());
    let length = 16 + 8 * segments;
    let seg_x2 = (2 * segments) as u16;
    let power = 1usize << (usize::BITS - 1 - segments.leading_zeros());
    let search = (2 * power) as u16;
    for v in [
        4,
        length as u16,
        0,
        seg_x2,
        search,
        power.trailing_zeros() as u16,
    ] {
        be16(&mut sub, v);
    }
    be16(&mut sub, seg_x2 - search);
    for &(code, _) in pairs {
        be16(&mut sub, code);
    }
    be16(&mut sub, 0xFFFF);
    be16(&mut sub, 0);
    for &(code, _) in pairs {
        be16(&mut sub, code);
    }
    be16(&mut sub, 0xFFFF);
    for &(code, gid) in pairs {
        be16(&mut sub, gid.wrapping_sub(code));
    }
    be16(&mut sub, 1);
    for _ in 0..segments {
        be16(&mut sub, 0);
    }
    let mut cmap = Vec::with_capacity(12 + sub.len());
    for v in [0u16, 1, 3, 1] {
        be16(&mut cmap, v);
    }
    cmap.extend_from_slice(&12u32.to_be_bytes());
    cmap.extend_from_slice(&sub);
    cmap
}

/// A DICT integer in its 5-byte form.
fn dict_int(out: &mut Vec<u8>, value: usize) -> Option<()> {
    let value = i32::try_from(value).ok()?;
    out.push(29);
    out.extend_from_slice(&value.to_be_bytes());
    Some(())
}

/// A CFF INDEX of `items`.
fn index(items: &[&[u8]]) -> Vec<u8> {
    let count = items.len() as u16;
    let mut out = count.to_be_bytes().to_vec();
    if items.is_empty() {
        return out;
    }
    let total: usize = items.iter().map(|item| item.len()).sum::<usize>() + 1;
    let size = match total {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    };
    out.push(size as u8);
    let mut offset = 1usize;
    let put = |out: &mut Vec<u8>, offset: usize| {
        out.extend_from_slice(&(offset as u32).to_be_bytes()[4 - size..]);
    };
    put(&mut out, offset);
    for item in items {
        offset += item.len();
        put(&mut out, offset);
    }
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

/// A Type 2 charstring drawing `commands` (closed contours; Type 2 closes
/// each one implicitly), without a width.
fn charstring(commands: &[BuiltinOutlineCommand], slant: f64) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut x, mut y) = (0i32, 0i32);
    let mut rel = |out: &mut Vec<u8>, px: i32, py: i32| {
        let px = px.saturating_add((f64::from(py) * slant).round() as i32);
        number(out, px.saturating_sub(x));
        number(out, py.saturating_sub(y));
        (x, y) = (px, py);
    };
    for command in commands {
        match *command {
            BuiltinOutlineCommand::MoveTo { x, y } => {
                rel(&mut out, x, y);
                out.push(21);
            }
            BuiltinOutlineCommand::LineTo { x, y } => {
                rel(&mut out, x, y);
                out.push(5);
            }
            BuiltinOutlineCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x,
                y,
            } => {
                rel(&mut out, x1, y1);
                rel(&mut out, x2, y2);
                rel(&mut out, x, y);
                out.push(8);
            }
            BuiltinOutlineCommand::Close => {}
        }
    }
    out.push(14);
    out
}

/// A Type 2 charstring operand.
fn number(out: &mut Vec<u8>, value: i32) {
    match value {
        -107..=107 => out.push((value + 139) as u8),
        108..=1131 => {
            let v = value - 108;
            out.extend_from_slice(&[(v / 256 + 247) as u8, (v % 256) as u8]);
        }
        -1131..=-108 => {
            let v = -value - 108;
            out.extend_from_slice(&[(v / 256 + 251) as u8, (v % 256) as u8]);
        }
        _ => {
            let v = value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
            out.push(28);
            out.extend_from_slice(&v.to_be_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_use_the_shortest_type2_form() {
        let encode = |v| {
            let mut out = Vec::new();
            number(&mut out, v);
            out
        };
        assert_eq!(encode(0), [139]);
        assert_eq!(encode(-107), [32]);
        assert_eq!(encode(108), [247, 0]);
        assert_eq!(encode(1131), [250, 255]);
        assert_eq!(encode(-1131), [254, 255]);
        assert_eq!(encode(2000), [28, 7, 208]);
        assert_eq!(encode(-40_000), [28, 0x80, 0]);
    }

    #[test]
    fn every_face_builds_and_the_empty_font_too() {
        for face in [
            Face::Helvetica,
            Face::HelveticaBold,
            Face::HelveticaOblique,
            Face::HelveticaBoldOblique,
            Face::TimesRoman,
        ] {
            let program = face.program().expect("face program");
            let bytes: &[u8] = (*program).as_ref();
            assert_eq!(bytes.get(..4), Some(b"OTTO".as_slice()), "{face:?}");
        }
        assert_eq!((*empty()).as_ref().get(..4), Some([1, 0, 4, 4].as_slice()));
        assert!(cff(b"x", &[], 0.0).is_none());
    }

    #[test]
    fn fonts_are_read_only_when_their_tables_are_within_the_data() {
        let mut font = b"\0\x01\0\0\0\x01\0\0\0\0\0\0".to_vec();
        font.extend_from_slice(b"cmap\0\0\0\0\0\0\0\x1c\0\0\0\x04abcd");
        let read = Sfnt::read(&font).expect("one table");
        assert_eq!(read.table(b"cmap"), Some(b"abcd".as_slice()));
        assert!(read.table(b"head").is_none());
        font.truncate(font.len() - 1);
        assert!(Sfnt::read(&font).is_none());
        assert!(Sfnt::read(b"OTTO").is_none());
        assert!(Sfnt::read(b"").is_none());
        assert_eq!(set_cjk_font(vec![0; 64]), Err(FontError::NotAFont));
        assert_eq!(
            set_cjk_font(vec![0; MAX_CJK_FONT_BYTES + 1]),
            Err(FontError::TooLarge)
        );
    }

    #[test]
    fn a_built_font_reads_back_with_its_cmap() {
        let program = Face::Helvetica.program().expect("Helvetica");
        let bytes: &[u8] = (*program).as_ref();
        let font = Sfnt::read(bytes).expect("sfnt");
        let gids = bmp_mapping(font.table(b"cmap").expect("cmap")).expect("mapping");
        let glyphs = glyphs_of(BuiltinFace::Helvetica);
        let a = usize::from(gids[0x410]);
        assert_eq!(glyphs[a].name, b"afii10017");
        assert_eq!(glyphs[usize::from(gids[usize::from(b'A')])].name, b"A");
        assert_eq!(gids[0x4E00], 0);
    }

    #[test]
    fn a_truetype_cjk_font_gets_an_empty_glyph_for_what_it_lacks() {
        // Two glyphs: .notdef and one mapped from U+65E5.
        let mut head = vec![0u8; 54];
        head[50..52].copy_from_slice(&1u16.to_be_bytes());
        let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
        maxp.extend_from_slice(&2u16.to_be_bytes());
        let pairs = [(0x65E5u16, 1u16)];
        let tables: [([u8; 4], &[u8]); 7] = [
            (*b"cmap", &cmap4(&pairs)),
            (*b"glyf", &[]),
            (*b"head", &head),
            (*b"hhea", &[0; 36]),
            (*b"hmtx", &[0; 8]),
            (*b"loca", &[0; 12]),
            (*b"maxp", &maxp),
        ];
        let bytes = sfnt(*b"\0\x01\0\0", &tables).expect("font");
        let total = with_total_cmap(&Sfnt::read(&bytes).expect("read")).expect("augmented");
        let font = Sfnt::read(&total).expect("read back");
        // The 653 URW glyphs the font lacks, the empty one, then the empty
        // block characters past the BMP map onto.
        let empty = 2 + 653;
        let count = empty + 1 + PAST_BMP_BLOCK;
        assert_eq!(u16_at(font.table(b"maxp").expect("maxp"), 4), Some(count));
        let loca = font.table(b"loca").expect("loca");
        assert_eq!(loca.len(), 4 * (usize::from(count) + 1));
        let gids = bmp_mapping(font.table(b"cmap").expect("cmap")).expect("mapping");
        let at = |gid: u16| u32_at(loca, 4 * usize::from(gid)).expect("offset");
        let a = gids[usize::from(b'A')];
        assert!(a >= 2 && at(a + 1) > at(a), "A is drawn");
        assert_eq!(
            at(count),
            at(empty),
            "the glyphs from the empty one on are empty"
        );
        assert_eq!(
            (gids[0x65E5], gids[0x4E00], gids[0xD800], gids[0xFFFF]),
            (1, empty, 0, 0)
        );
    }

    /// The glyph the augmented font's format 12 subtable maps `c` to.
    fn map12(font: &Sfnt<'_>, c: u32) -> Option<u16> {
        let cmap = font.table(b"cmap")?;
        let sub = cmap.get(usize::try_from(u32_at(cmap, 8)?).ok()?..)?;
        (0..usize::try_from(u32_at(sub, 12)?).ok()?).find_map(|g| {
            let at = 16 + 12 * g;
            let (first, last) = (u32_at(sub, at)?, u32_at(sub, at + 4)?);
            let gid = u32_at(sub, at + 8)?;
            (first..=last)
                .contains(&c)
                .then(|| u16::try_from(gid + (c - first)).ok())?
        })
    }

    #[test]
    fn characters_past_the_bmp_keep_the_fonts_glyphs_and_else_map_to_empty_ones() {
        // Three glyphs; a UCS-4 subtable maps U+65E5 to 1 and U+20001 to 2,
        // and a run from U+2FFFF that reaches past the font's glyphs.
        let mut head = vec![0u8; 54];
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[50..52].copy_from_slice(&1u16.to_be_bytes());
        let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
        maxp.extend_from_slice(&3u16.to_be_bytes());
        let mut hhea = vec![0u8; 36];
        hhea[34..36].copy_from_slice(&3u16.to_be_bytes());
        let groups = [
            (0x65E5u32, 0x65E5u32, 1u32),
            (0x2_0001, 0x2_0001, 2),
            (0x2_FFFF, 0x3_0005, 1),
        ];
        let mut cmap: Vec<u8> = [0u16, 1, 3, 10]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        cmap.extend_from_slice(&12u32.to_be_bytes());
        cmap.extend_from_slice(&12u16.to_be_bytes());
        cmap.extend_from_slice(&0u16.to_be_bytes());
        for v in [16 + 12 * groups.len() as u32, 0, groups.len() as u32] {
            cmap.extend_from_slice(&v.to_be_bytes());
        }
        for (first, last, gid) in groups {
            for v in [first, last, gid] {
                cmap.extend_from_slice(&v.to_be_bytes());
            }
        }
        let tables: [([u8; 4], &[u8]); 7] = [
            (*b"cmap", &cmap),
            (*b"glyf", &[]),
            (*b"head", &head),
            (*b"hhea", &hhea),
            (*b"hmtx", &[0; 12]),
            (*b"loca", &[0; 16]),
            (*b"maxp", &maxp),
        ];
        let bytes = sfnt(*b"\0\x01\0\0", &tables).expect("font");
        let total = with_total_cmap(&Sfnt::read(&bytes).expect("read")).expect("augmented");
        let font = Sfnt::read(&total).expect("read back");
        let count = u16_at(font.table(b"maxp").expect("maxp"), 4).expect("count");
        let block = count - PAST_BMP_BLOCK..count;
        assert_eq!(map12(&font, 0x65E5), Some(1));
        assert_eq!(map12(&font, 0x2_0001), Some(2), "the font's own");
        // The run keeps the glyphs the font has (1 and 2), then is cut.
        assert_eq!(
            (map12(&font, 0x2_FFFF), map12(&font, 0x3_0000)),
            (Some(1), Some(2))
        );
        for c in [0x1_0000, 0x2_0000, 0x2_0002, 0x3_0001, 0xE_0001, 0x10_FFFF] {
            let glyph = map12(&font, c).expect("mapped");
            assert!(block.contains(&glyph), "U+{c:X} to {glyph}");
        }
        let loca = font.table(b"loca").expect("loca");
        let at = |gid: u16| u32_at(loca, 4 * usize::from(gid)).expect("offset");
        assert_eq!(at(block.start), at(count), "the block is empty");
        assert_eq!(
            font.table(b"hmtx").map(<[u8]>::len),
            Some(4 * usize::from(count))
        );
    }

    #[test]
    fn appended_glyphs_keep_urw_advances_in_long_metrics() {
        // Three glyphs at 2,000 units per em: .notdef and U+65E5 with long
        // metrics (advances 1,000 and 2,000), then U+672C with a bearing only.
        let mut head = vec![0u8; 54];
        head[18..20].copy_from_slice(&2000u16.to_be_bytes());
        head[50..52].copy_from_slice(&1u16.to_be_bytes());
        let mut maxp = 0x0000_5000u32.to_be_bytes().to_vec();
        maxp.extend_from_slice(&3u16.to_be_bytes());
        let mut hhea = vec![0u8; 36];
        hhea[10..12].copy_from_slice(&2000u16.to_be_bytes());
        hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
        let hmtx: Vec<u8> = [1000u16, 10, 2000, 20, 30]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        let tables: [([u8; 4], &[u8]); 7] = [
            (*b"cmap", &cmap4(&[(0x65E5, 1), (0x672C, 2)])),
            (*b"glyf", &[]),
            (*b"head", &head),
            (*b"hhea", &hhea),
            (*b"hmtx", &hmtx),
            (*b"loca", &[0; 16]),
            (*b"maxp", &maxp),
        ];
        let bytes = sfnt(*b"\0\x01\0\0", &tables).expect("font");
        let total = with_total_cmap(&Sfnt::read(&bytes).expect("read")).expect("augmented");
        let font = Sfnt::read(&total).expect("read back");
        let count = u16_at(font.table(b"maxp").expect("maxp"), 4).expect("count");
        let hhea = font.table(b"hhea").expect("hhea");
        assert_eq!(u16_at(hhea, 34), Some(count), "every metric long");
        let hmtx = font.table(b"hmtx").expect("hmtx");
        assert_eq!(hmtx.len(), 4 * usize::from(count));
        let metric = |gid: u16| {
            let at = 4 * usize::from(gid);
            (
                u16_at(hmtx, at).expect("advance"),
                u16_at(hmtx, at + 2).expect("bearing"),
            )
        };
        // The font's own glyphs keep theirs; a short one takes the last advance.
        assert_eq!(
            (metric(0), metric(1), metric(2)),
            ((1000, 10), (2000, 20), (2000, 30))
        );
        let gids = bmp_mapping(font.table(b"cmap").expect("cmap")).expect("mapping");
        let urw = glyphs_of(BuiltinFace::Helvetica);
        for c in *b"Aim1" {
            let advance = urw
                .iter()
                .find(|g| g.unicode == Some(u32::from(c)))
                .expect("URW glyph")
                .advance;
            assert_eq!(
                metric(gids[usize::from(c)]).0,
                2 * advance,
                "{}",
                char::from(c)
            );
        }
        assert_eq!(
            metric(count - 1).0,
            2000,
            "the empty glyph keeps the last advance"
        );
        let widest = (0..count).map(|gid| metric(gid).0).max();
        assert_eq!(u16_at(hhea, 10), widest, "advanceWidthMax");
        assert!(widest > Some(2000));
    }
}
