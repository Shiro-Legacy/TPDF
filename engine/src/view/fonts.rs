// SPDX-License-Identifier: AGPL-3.0-or-later
//! The fonts the view gives hayro for text whose font program is not
//! embedded. The base 14 fonts draw with URW's outlines; the faces whose URW
//! outlines the engine carries (`builtin_outlines`: Helvetica, Helvetica-Bold,
//! Helvetica-BoldOblique, Times-Roman, and Helvetica-Oblique as Helvetica
//! slanted 12°, which URW's is) are built into OpenType fonts with CFF
//! outlines on first use, so their Cyrillic, Greek and other non-Latin
//! glyphs draw. The other faces keep hayro's Foxit fonts, Latin only.
//!
//! CID fonts of a Chinese, Japanese or Korean collection draw with a font the
//! host installs (`set_cjk_font`, such as Droid Sans Fallback), loaded on
//! demand. Until one is installed they get a font without glyphs, so they draw
//! nothing (hayro would otherwise read their two-byte codes as Helvetica text)
//! while their text is still extracted, and `cjk_font_wanted` turns true. A
//! TrueType font is rebuilt per collection on its first use (`by_cid`), so
//! each CID draws the collection's character whether or not hayro finds its
//! Unicode.
use crate::builtin_metrics::BuiltinFace;
use crate::builtin_outlines::{BuiltinOutline, BuiltinOutlineCommand};
use hayro_interpret::font::{FallbackFontQuery, FontData, FontQuery, StandardFont};
use hayro_interpret::hayro_cmap::{BfString, CMap, CidFamily, load_embedded};
use hayro_interpret::{FontResolverFn, InterpreterSettings};
use std::collections::HashMap;
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

/// A CID font without its program: for a CJK collection the installed
/// font as built for that collection (`by_cid`), else the base 14 font
/// hayro picks from the descriptor.
fn fallback(query: &FallbackFontQuery, paint: bool) -> (FontData, u32) {
    let collection = (query.character_collection.as_ref())
        .and_then(|c| COLLECTIONS.iter().position(|family| *family == c.family));
    let Some(collection) = collection else {
        return standard(query.pick_standard_font());
    };
    let mut installed = CJK.lock().unwrap_or_else(PoisonError::into_inner);
    match installed.as_mut() {
        Some(installed) => (installed.font_for(collection), 0),
        None => {
            if paint && !CJK_REFUSED.load(Ordering::Relaxed) {
                CJK_WANTED.store(true, Ordering::Relaxed);
            }
            (empty(), 0)
        }
    }
}

/// The CJK collections, in the order of `Installed::built`.
const COLLECTIONS: [CidFamily; 4] = [
    CidFamily::AdobeGB1,
    CidFamily::AdobeCNS1,
    CidFamily::AdobeJapan1,
    CidFamily::AdobeKorea1,
];

/// The font the host installed and, per collection, the font built from
/// it on that collection's first use.
struct Installed {
    font: FontData,
    built: [Option<FontData>; 4],
}

impl Installed {
    /// The font a CID font of `COLLECTIONS[collection]` draws with: built
    /// once (`by_cid`), or the installed font as it is when that fails (a
    /// CFF-flavoured font, a font past the limits).
    fn font_for(&mut self, collection: usize) -> FontData {
        let Self { font, built } = self;
        let (Some(slot), Some(family)) = (built.get_mut(collection), COLLECTIONS.get(collection))
        else {
            return font.clone();
        };
        slot.get_or_insert_with(|| {
            let bytes: &[u8] = (**font).as_ref();
            by_cid(bytes, family).map_or_else(|| font.clone(), |built| Arc::new(built))
        })
        .clone()
    }
}

static CJK: Mutex<Option<Installed>> = Mutex::new(None);
static CJK_WANTED: AtomicBool = AtomicBool::new(false);
/// The last `set_cjk_font` refused its font: renders no longer ask for one.
static CJK_REFUSED: AtomicBool = AtomicBool::new(false);

/// Whether hayro took `font` from the CJK fallback (the installed font, a
/// font built from it, or the empty one): text extraction then measures a
/// glyph by its em box, not its outline, so what it keeps of a clipped run
/// is the same whether or not a font was installed.
pub(crate) fn is_cjk_fallback(font: &FontData) -> bool {
    let same = |other: &FontData| std::ptr::addr_eq(Arc::as_ptr(font), Arc::as_ptr(other));
    if same(&empty()) {
        return true;
    }
    let installed = CJK.lock().unwrap_or_else(PoisonError::into_inner);
    installed
        .as_ref()
        .is_some_and(|i| same(&i.font) || i.built.iter().flatten().any(same))
}

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
/// Droid Sans Fallback. A TrueType font is rebuilt per collection
/// on first use (`by_cid`) so that hayro draws every CID as the
/// collection's character, URW Helvetica's glyph where the font lacks it,
/// or nothing; any other font is used as it is.
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
    *CJK.lock().unwrap_or_else(PoisonError::into_inner) = Some(Installed {
        font: Arc::new(bytes),
        built: Default::default(),
    });
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

/// Glyphs a font built for a collection may hold.
const MAX_GLYPHS: usize = 65_535;

/// Where a glyph of a font built for a collection comes from.
#[derive(Clone, Copy)]
enum Source {
    Empty,
    /// The installed font's glyph.
    Own(u16),
    /// URW Helvetica's glyph (`glyphs_of`).
    Urw(usize),
}

/// The installed TrueType font rebuilt for a CJK collection, so that hayro
/// draws a CID font without its program by CID: glyph `i`
/// is the font's glyph for the character the collection's UCS2 CMap gives
/// CID `i`, URW Helvetica's (at URW's advance) where the font lacks it
/// (a CJK font's missing Latin, digits and symbols still draw), else
/// empty. The cmap, one format 12 subtable,
/// maps each of those characters to its glyph. hayro finds a glyph by the
/// code's Unicode through the cmap and otherwise (no ToUnicode entry,
/// several characters, U+FFFF, a lone surrogate, a character outside the
/// collection) takes glyph CID, so both routes draw the collection's glyph
/// and a CID past the collection draws nothing. So that the font has no
/// glyph but the CIDs', composite glyphs are flattened into simple ones. Every glyph has a long horizontal metric
/// (hayro stretches a fallback glyph to its /W width by its advance). None
/// for a font that is not TrueType, a collection without a UCS2 CMap, or
/// past four times the font's glyph data or `MAX_FLATTEN_WORK`.
fn by_cid(bytes: &[u8], family: &CidFamily) -> Option<Vec<u8>> {
    let font = Sfnt::read(bytes)?;
    let (head, maxp, loca, glyf, hmtx, hhea, cmap) = (
        font.table(b"head")?,
        font.table(b"maxp")?,
        font.table(b"loca")?,
        font.table(b"glyf")?,
        font.table(b"hmtx")?,
        font.table(b"hhea")?,
        font.table(b"cmap")?,
    );
    let glyphs = u16_at(maxp, 4)?;
    let units = f64::from(u16_at(head, 18)?.clamp(16, 16_384)) / 1000.0;
    let long_loca = u16_at(head, 50)? == 1;
    let long_metrics = u16_at(hhea, 34)?.min(glyphs);
    let last = match long_metrics.checked_sub(1) {
        Some(i) => u16_at(hmtx, 4 * usize::from(i))?,
        None => 0,
    };
    let mut bmp = bmp_mapping(cmap)?;
    for gid in bmp.iter_mut().filter(|gid| **gid >= glyphs) {
        *gid = 0;
    }
    let planes = ucs4_runs(cmap, glyphs).unwrap_or_default();
    let own = |c: char| -> Option<u16> {
        let c = u32::from(c);
        let gid = match bmp.get(c as usize) {
            Some(&gid) => gid,
            None => {
                let run = planes.partition_point(|&(_, last, _)| last < c);
                let &(first, _, gid) = planes.get(run).filter(|run| run.0 <= c)?;
                gid.checked_add(u16::try_from(c - first).ok()?)?
            }
        };
        (gid != 0).then_some(gid)
    };
    let urw = glyphs_of(BuiltinFace::Helvetica);
    let mut urw_of: HashMap<u32, usize> = HashMap::new();
    for (i, glyph) in urw.iter().enumerate().skip(1) {
        if let Some(c) = glyph.unicode {
            urw_of.entry(c).or_insert(i);
        }
    }
    let ucs2 = CMap::parse(load_embedded(family.ucs2_cmap()?)?, load_embedded)?;
    let chars: Vec<Option<char>> = (0..=u32::from(u16::MAX))
        .map(|cid| collection_char(ucs2.lookup_bf_string(cid)))
        .collect();
    let cids = (chars.iter().rposition(Option::is_some)? + 1).min(MAX_GLYPHS);
    let sources: Vec<Source> = chars[..cids]
        .iter()
        .map(|c| match *c {
            Some(c) => own(c)
                .map(Source::Own)
                .or_else(|| urw_of.get(&u32::from(c)).map(|&i| Source::Urw(i)))
                .unwrap_or(Source::Empty),
            None => Source::Empty,
        })
        .collect();
    let budget = glyf.len().saturating_mul(4).saturating_add(1 << 20);
    let mut data: Vec<u8> = Vec::new();
    let mut offsets: Vec<u8> = Vec::with_capacity(4 * (sources.len() + 1));
    let mut horizontal: Vec<u8> = Vec::with_capacity(4 * sources.len());
    // Where the first copy of each of the font's glyphs is in `data`.
    let mut copied: Vec<Option<(usize, usize)>> = vec![None; usize::from(glyphs)];
    let mut converted: HashMap<usize, Option<(Vec<u8>, u16, u16)>> = HashMap::new();
    let (mut points, mut contours, mut widest) = (0u16, 0u16, last);
    let (mut outline, mut work) = (Points::default(), 0usize);
    data.reserve(glyf.len().saturating_add(glyf.len() / 2));
    for &source in &sources {
        offsets.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
        let (advance, bearing) = match source {
            Source::Empty => (last, [0, 0]),
            Source::Own(g) => {
                let start = data.len();
                match copied.get(usize::from(g)).copied().flatten() {
                    Some((from, to)) => data.extend_from_within(from..to),
                    None => {
                        let bytes = glyph_data(glyf, loca, long_loca, g);
                        if bytes.first().is_some_and(|n| n & 0x80 != 0) {
                            // A composite, flattened: its components would
                            // need glyph ids, all of which are CIDs.
                            outline.clear();
                            let read =
                                outline.composite(glyf, loca, long_loca, bytes, 4, &mut work);
                            if work > MAX_FLATTEN_WORK {
                                return None;
                            }
                            if let Some((n, c)) = read.and_then(|()| outline.write(&mut data)) {
                                (points, contours) = (points.max(n), contours.max(c));
                            } else {
                                data.truncate(start);
                            }
                        } else {
                            data.extend_from_slice(bytes);
                        }
                        if let Some(slot) = copied.get_mut(usize::from(g)) {
                            *slot = Some((start, data.len()));
                        }
                    }
                }
                own_metric(hmtx, long_metrics, last, g)
            }
            Source::Urw(i) => {
                let glyph = urw.get(i)?;
                let advance = (f64::from(glyph.advance) * units).round() as u16;
                let outline = converted
                    .entry(i)
                    .or_insert_with(|| truetype_glyph(glyph.commands, units));
                let mut bearing = [0, 0];
                if let Some((bytes, n, c)) = outline {
                    data.extend_from_slice(bytes);
                    bearing.copy_from_slice(bytes.get(2..4).unwrap_or(&[0, 0]));
                    (points, contours) = (points.max(*n), contours.max(*c));
                }
                (advance, bearing)
            }
        };
        if data.len() > budget {
            return None;
        }
        widest = widest.max(advance);
        horizontal.extend_from_slice(&advance.to_be_bytes());
        horizontal.extend_from_slice(&bearing);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    offsets.extend_from_slice(&u32::try_from(data.len()).ok()?.to_be_bytes());
    let count = u16::try_from(sources.len()).ok()?;
    // Each character the collection's glyphs draw, to its first glyph.
    let mut pairs: Vec<(u32, u16)> = Vec::new();
    for (at, (c, source)) in chars.iter().zip(&sources).take(cids).enumerate() {
        if let (Some(c), Source::Own(_) | Source::Urw(_)) = (c, source) {
            pairs.push((u32::from(*c), u16::try_from(at).ok()?));
        }
    }
    pairs.sort_unstable();
    pairs.dedup_by_key(|pair| pair.0);
    let mut groups: Vec<(u32, u32, u16)> = Vec::new();
    for (c, gid) in pairs {
        if let Some((first, last, first_gid)) = groups.last_mut()
            && *last + 1 == c
            && u32::from(*first_gid) + (c - *first) == u32::from(gid)
        {
            *last = c;
            continue;
        }
        groups.push((c, c, gid));
    }
    let cmap = cmap12(&groups)?;
    let mut head = head.to_vec();
    head.get_mut(50..52)?.copy_from_slice(&1u16.to_be_bytes());
    let mut maxp = maxp.to_vec();
    maxp.get_mut(4..6)?.copy_from_slice(&count.to_be_bytes());
    if maxp.len() >= 10 {
        // Version 1.0: maxPoints and maxContours cover the glyphs written.
        for (at, v) in [(6, points), (8, contours)] {
            let old = u16_at(&maxp, at)?;
            maxp.get_mut(at..at + 2)?
                .copy_from_slice(&old.max(v).to_be_bytes());
        }
    }
    let mut hhea = hhea.to_vec();
    hhea.get_mut(10..12)?.copy_from_slice(&widest.to_be_bytes());
    hhea.get_mut(34..36)?.copy_from_slice(&count.to_be_bytes());
    // post 3.0: no glyph names, which would be by the font's glyph ids.
    let post = (font.table(b"post"))
        .and_then(|post| post.get(..32))
        .map(|post| [&0x0003_0000u32.to_be_bytes()[..], &post[4..]].concat());
    let mut tables: Vec<([u8; 4], &[u8])> = vec![
        (*b"cmap", &cmap),
        (*b"glyf", &data),
        (*b"head", &head),
        (*b"hhea", &hhea),
        (*b"hmtx", &horizontal),
        (*b"loca", &offsets),
        (*b"maxp", &maxp),
    ];
    if let Some(post) = &post {
        tables.push((*b"post", post));
    }
    // Tables that do not index glyphs; GSUB, GPOS, vmtx and the like do.
    for tag in [b"OS/2", b"cvt ", b"fpgm", b"gasp", b"name", b"prep"] {
        if let Some(table) = font.table(tag) {
            tables.push((*tag, table));
        }
    }
    tables.sort_by_key(|(tag, _)| *tag);
    sfnt(font.version, &tables)
}

/// The character a CID of a collection draws: its UCS2 character, or the
/// base of a variation sequence; none for any other sequence (a fraction,
/// two digits in one glyph), which reference viewers leave blank.
fn collection_char(unicode: Option<BfString>) -> Option<char> {
    let selector = |c: char| matches!(u32::from(c), 0xFE00..=0xFE0F | 0xE_0100..=0xE_01EF);
    match unicode? {
        BfString::Char(c) => Some(c),
        BfString::String(s) => {
            let mut chars = s.chars();
            let base = chars.next()?;
            chars.all(selector).then_some(base)
        }
    }
}

/// A glyph's data in `glyf`; empty when `loca` says nothing valid.
fn glyph_data<'a>(glyf: &'a [u8], loca: &[u8], long: bool, glyph: u16) -> &'a [u8] {
    let offset = |i: usize| -> Option<usize> {
        if long {
            usize::try_from(u32_at(loca, 4 * i)?).ok()
        } else {
            Some(2 * usize::from(u16_at(loca, 2 * i)?))
        }
    };
    let glyph = usize::from(glyph);
    (offset(glyph).zip(offset(glyph + 1)))
        .and_then(|(start, end)| glyf.get(start..end))
        .unwrap_or_default()
}

/// A glyph's advance and left side bearing (as stored) in `hmtx`; past its
/// long metrics, the last long advance.
fn own_metric(hmtx: &[u8], long: u16, last: u16, glyph: u16) -> (u16, [u8; 2]) {
    let (glyph, long) = (usize::from(glyph), usize::from(long));
    let (advance, bearing) = if glyph < long {
        (u16_at(hmtx, 4 * glyph).unwrap_or(last), 4 * glyph + 2)
    } else {
        (last, 4 * long + 2 * (glyph - long))
    };
    let bearing = u16_at(hmtx, bearing).unwrap_or(0).to_be_bytes();
    (advance, bearing)
}

/// Points and component records flattening a collection's composite glyphs
/// may read in all.
const MAX_FLATTEN_WORK: usize = 1 << 24;

fn i16_at(bytes: &[u8], at: usize) -> Option<i16> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(i16::from_be_bytes([b[0], b[1]]))
}

/// A glyph's contours: points (x, y, on the curve) and each contour's last;
/// with buffers kept from glyph to glyph.
#[derive(Default)]
struct Points {
    points: Vec<(i32, i32, bool)>,
    ends: Vec<usize>,
    flags: Vec<u8>,
    xs: Vec<u8>,
    ys: Vec<u8>,
}

impl Points {
    fn clear(&mut self) {
        self.points.clear();
        self.ends.clear();
    }

    /// Adds a glyph's contours, its components resolved to `depth` levels;
    /// `work` counts the points and records read. None for data that is not
    /// a glyph.
    fn add(
        &mut self,
        glyf: &[u8],
        loca: &[u8],
        long: bool,
        glyph: &[u8],
        depth: u8,
        work: &mut usize,
    ) -> Option<()> {
        match glyph.first() {
            None => Some(()),
            Some(n) if n & 0x80 != 0 => self.composite(glyf, loca, long, glyph, depth, work),
            Some(_) => self.simple(glyph, work),
        }
    }

    fn simple(&mut self, glyph: &[u8], work: &mut usize) -> Option<()> {
        let contours = usize::from(u16_at(glyph, 0)?);
        let base = self.points.len();
        let mut last = None;
        for i in 0..contours {
            let end = usize::from(u16_at(glyph, 10 + 2 * i)?);
            if last.is_some_and(|last| end < last) {
                return None;
            }
            last = Some(end);
        }
        let Some(n) = last.map(|end| end + 1) else {
            return Some(());
        };
        *work = work.saturating_add(n);
        if *work > MAX_FLATTEN_WORK {
            return None;
        }
        let mut at = 12 + 2 * contours + usize::from(u16_at(glyph, 10 + 2 * contours)?);
        self.flags.clear();
        while self.flags.len() < n {
            let flag = *glyph.get(at)?;
            at += 1;
            let mut copies = 1;
            if flag & 0x08 != 0 {
                copies += usize::from(*glyph.get(at)?);
                at += 1;
            }
            let copies = copies.min(n - self.flags.len());
            self.flags.extend(std::iter::repeat_n(flag, copies));
        }
        // x, then y: a byte (its sign in the same bit), none (same), or a
        // word; at most 65,535 deltas of 16 bits.
        let mut v = 0i32;
        for &flag in &self.flags {
            if flag & 0x02 != 0 {
                let d = i32::from(*glyph.get(at)?);
                at += 1;
                v = v.saturating_add(if flag & 0x10 != 0 { d } else { -d });
            } else if flag & 0x10 == 0 {
                v = v.saturating_add(i32::from(i16_at(glyph, at)?));
                at += 2;
            }
            self.points.push((v, 0, flag & 1 != 0));
        }
        v = 0;
        for (flag, point) in self.flags.iter().zip(&mut self.points[base..]) {
            if flag & 0x04 != 0 {
                let d = i32::from(*glyph.get(at)?);
                at += 1;
                v = v.saturating_add(if flag & 0x20 != 0 { d } else { -d });
            } else if flag & 0x20 == 0 {
                v = v.saturating_add(i32::from(i16_at(glyph, at)?));
                at += 2;
            }
            point.1 = v;
        }
        for i in 0..contours {
            self.ends
                .push(base + usize::from(u16_at(glyph, 10 + 2 * i)?));
        }
        Some(())
    }

    /// A composite glyph's components, each moved and scaled (TrueType's
    /// argument and transform flags).
    fn composite(
        &mut self,
        glyf: &[u8],
        loca: &[u8],
        long: bool,
        glyph: &[u8],
        depth: u8,
        work: &mut usize,
    ) -> Option<()> {
        let depth = depth.checked_sub(1)?;
        let mut at = 10;
        loop {
            *work = work.saturating_add(1);
            if *work > MAX_FLATTEN_WORK {
                return None;
            }
            let flags = u16_at(glyph, at)?;
            let component = u16_at(glyph, at + 2)?;
            at += 4;
            // Offsets (signed) or point numbers (unsigned), words or bytes.
            let xy = flags & 0x0002 != 0;
            let (a1, a2) = if flags & 0x0001 != 0 {
                at += 4;
                if xy {
                    (
                        i32::from(i16_at(glyph, at - 4)?),
                        i32::from(i16_at(glyph, at - 2)?),
                    )
                } else {
                    (
                        i32::from(u16_at(glyph, at - 4)?),
                        i32::from(u16_at(glyph, at - 2)?),
                    )
                }
            } else {
                let (p, q) = (*glyph.get(at)?, *glyph.get(at + 1)?);
                at += 2;
                if xy {
                    (
                        i32::from(i8::from_be_bytes([p])),
                        i32::from(i8::from_be_bytes([q])),
                    )
                } else {
                    (i32::from(p), i32::from(q))
                }
            };
            let f2dot14 = |at: usize| i16_at(glyph, at).map(|v| f64::from(v) / 16384.0);
            let (a, b, c, d) = if flags & 0x0008 != 0 {
                let s = f2dot14(at)?;
                at += 2;
                (s, 0.0, 0.0, s)
            } else if flags & 0x0040 != 0 {
                let (x, y) = (f2dot14(at)?, f2dot14(at + 2)?);
                at += 4;
                (x, 0.0, 0.0, y)
            } else if flags & 0x0080 != 0 {
                let m = (
                    f2dot14(at)?,
                    f2dot14(at + 2)?,
                    f2dot14(at + 4)?,
                    f2dot14(at + 6)?,
                );
                at += 8;
                m
            } else {
                (1.0, 0.0, 0.0, 1.0)
            };
            // The component's points go after the glyph's so far, then move.
            let base = self.points.len();
            let data = glyph_data(glyf, loca, long, component);
            self.add(glyf, loca, long, data, depth, work)?;
            let round = |v: f64| v.round().clamp(-1e9, 1e9) as i32;
            let part = self.points.get_mut(base..)?;
            if (a, b, c, d) != (1.0, 0.0, 0.0, 1.0) {
                for p in part.iter_mut() {
                    let (x, y) = (f64::from(p.0), f64::from(p.1));
                    (p.0, p.1) = (round(a * x + c * y), round(b * x + d * y));
                }
            }
            let (dx, dy) = if xy {
                if flags & 0x0800 != 0 {
                    let (x, y) = (f64::from(a1), f64::from(a2));
                    (round(a * x + c * y), round(b * x + d * y))
                } else {
                    (a1, a2)
                }
            } else {
                // Point a1 of the glyph so far meets point a2 of the component.
                let p = (self.points[..base]).get(usize::try_from(a1).ok()?)?;
                let q = self.points.get(base + usize::try_from(a2).ok()?)?;
                (p.0.saturating_sub(q.0), p.1.saturating_sub(q.1))
            };
            for p in self.points.get_mut(base..)? {
                (p.0, p.1) = (p.0.saturating_add(dx), p.1.saturating_add(dy));
            }
            if flags & 0x0020 == 0 {
                return Some(());
            }
        }
    }

    /// Writes the contours to `out` as a simple TrueType glyph (coordinates
    /// rounded; nothing for no contours); their point and contour counts.
    /// None past the format's counts.
    fn write(&mut self, out: &mut Vec<u8>) -> Option<(u16, u16)> {
        if self.points.is_empty() {
            return Some((0, 0));
        }
        let n = u16::try_from(self.points.len()).ok()?;
        let contours = i16::try_from(self.ends.len()).ok()?;
        let clamp = |v: i32| v.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        let (mut x0, mut y0, mut x1, mut y1) = (i16::MAX, i16::MAX, i16::MIN, i16::MIN);
        for &(x, y, _) in &self.points {
            let (x, y) = (clamp(x), clamp(y));
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        for v in [contours, x0, y0, x1, y1] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        for &end in &self.ends {
            out.extend_from_slice(&u16::try_from(end).ok()?.to_be_bytes());
        }
        out.extend_from_slice(&0u16.to_be_bytes());
        // On the curve or not; a delta under 256 as a byte with its sign in
        // the same bit, none for 0, else a word.
        let (flags, xs, ys) = (&mut self.flags, &mut self.xs, &mut self.ys);
        flags.clear();
        xs.clear();
        ys.clear();
        let (mut last_x, mut last_y) = (0i16, 0i16);
        for &(x, y, on) in &self.points {
            let mut flag = u8::from(on);
            for (v, last, out, short, same) in [
                (clamp(x), &mut last_x, &mut *xs, 0x02, 0x10),
                (clamp(y), &mut last_y, &mut *ys, 0x04, 0x20),
            ] {
                let delta = v.wrapping_sub(*last);
                *last = v;
                if delta == 0 {
                    flag |= same;
                } else if delta.unsigned_abs() < 256 {
                    flag |= short | if delta > 0 { same } else { 0 };
                    out.push(delta.unsigned_abs() as u8);
                } else {
                    out.extend_from_slice(&delta.to_be_bytes());
                }
            }
            flags.push(flag);
        }
        out.extend_from_slice(flags);
        out.extend_from_slice(xs);
        out.extend_from_slice(ys);
        Some((n, contours.unsigned_abs()))
    }
}

/// One cmap of one format 12 subtable (3, 10) of `groups`.
fn cmap12(groups: &[(u32, u32, u16)]) -> Option<Vec<u8>> {
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
    for (first, last, gid) in groups {
        for v in [*first, *last, u32::from(*gid)] {
            cmap.extend_from_slice(&v.to_be_bytes());
        }
    }
    Some(cmap)
}

/// The runs past the BMP a cmap's UCS-4 subtable maps, sorted, overlapping
/// runs dropped, each cut to the font's `glyphs`. None past 2^16 groups.
fn ucs4_runs(cmap: &[u8], glyphs: u16) -> Option<Vec<(u32, u32, u16)>> {
    let count = usize::from(u16_at(cmap, 2)?);
    let mut runs: Vec<(u32, u32, u16)> = Vec::new();
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
            runs.push((
                first,
                last.min(first + (room - 1)),
                u16::try_from(glyph).ok()?,
            ));
        }
        break;
    }
    runs.sort_unstable();
    let mut next = 0;
    runs.retain(|&(first, last, _)| {
        let keep = first >= next;
        if keep {
            next = last + 1;
        }
        keep
    });
    Some(runs)
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

#[path = "fonts_scan.rs"]
pub(crate) mod scan;

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
    cff_program(name, glyphs, slant, &CffExtras::default())
}

/// What an embedded CFF program carries beyond `cff`'s: each glyph's
/// advance (1/1000 em; none when empty) and the Top DICT's Copyright and
/// Notice strings.
#[derive(Default)]
pub(crate) struct CffExtras<'a> {
    pub widths: &'a [u16],
    pub copyright: Option<&'a [u8]>,
    pub notice: Option<&'a [u8]>,
}

/// `cff` with `extras`: the most common advance is defaultWidthX (and
/// nominalWidthX), and every other glyph's charstring starts with its
/// advance less nominalWidthX (Type 2's optional width operand).
fn cff_program(
    name: &[u8],
    glyphs: &[(&[u8], &[BuiltinOutlineCommand])],
    slant: f64,
    extras: &CffExtras<'_>,
) -> Option<Vec<u8>> {
    let count = glyphs.len();
    if count == 0 || count > usize::from(u16::MAX) || FIRST_CUSTOM_SID + count + 2 > 65_000 {
        return None;
    }
    let widths = extras.widths;
    if !widths.is_empty() && widths.len() != count {
        return None;
    }
    let mut tally: HashMap<u16, usize> = HashMap::new();
    for &width in widths {
        *tally.entry(width).or_default() += 1;
    }
    let default = (tally.iter())
        .max_by_key(|&(width, n)| (*n, std::cmp::Reverse(*width)))
        .map_or(0, |(width, _)| *width);
    let charstrings: Vec<Vec<u8>> = (glyphs.iter().enumerate())
        .map(|(i, (_, commands))| {
            let mut out = Vec::new();
            if let Some(&width) = widths.get(i).filter(|&&width| width != default) {
                number(&mut out, i32::from(width) - i32::from(default));
            }
            out.extend_from_slice(&charstring(commands, slant));
            out
        })
        .collect();
    let mut names: Vec<&[u8]> = glyphs.iter().skip(1).map(|(name, _)| *name).collect();
    let glyph_names = names.len();
    let mut strings = Vec::new();
    for (text, op) in [
        (extras.notice, &[1u8][..]),
        (extras.copyright, &[12u8, 0][..]),
    ] {
        if let Some(text) = text {
            strings.push((u16::try_from(FIRST_CUSTOM_SID + names.len()).ok()?, op));
            names.push(text);
        }
    }
    let name_index = index(&[name]);
    let string_index = index(&names);
    let global_subrs = index(&[]);
    let mut charset = vec![0u8];
    for i in 0..glyph_names {
        let sid = u16::try_from(FIRST_CUSTOM_SID + i).ok()?;
        charset.extend_from_slice(&sid.to_be_bytes());
    }
    let refs: Vec<&[u8]> = charstrings.iter().map(Vec::as_slice).collect();
    let charstring_index = index(&refs);
    let private = if widths.is_empty() {
        // defaultWidthX 0, nominalWidthX 0.
        vec![139, 20, 139, 21]
    } else {
        let mut private = Vec::with_capacity(12);
        for op in [20u8, 21] {
            dict_int(&mut private, usize::from(default))?;
            private.push(op);
        }
        private
    };
    // The Top DICT's integers are 5-byte ones, so its size is fixed.
    let strings_len: usize = strings.iter().map(|(_, op)| 5 + op.len()).sum();
    let top_len = 3 * 6 + 5 + strings_len;
    let top_index_len = 2 + 1 + 2 + top_len;
    let charset_at = 4 + name_index.len() + top_index_len + string_index.len() + global_subrs.len();
    let charstrings_at = charset_at + charset.len();
    let private_at = charstrings_at + charstring_index.len();
    let mut top = Vec::with_capacity(top_len);
    for (sid, op) in &strings {
        dict_int(&mut top, usize::from(*sid))?;
        top.extend_from_slice(op);
    }
    for (value, op) in [(charset_at, 15u8), (charstrings_at, 17)] {
        dict_int(&mut top, value)?;
        top.push(op);
    }
    dict_int(&mut top, private.len())?;
    dict_int(&mut top, private_at)?;
    top.push(18);
    if top.len() != top_len || top_len + 1 > 0xFF {
        return None;
    }
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

    /// A TrueType font at 1,000 units per em of `glyphs` (long loca), the
    /// first `long` with long metrics from `metrics` (advance, bearing) and
    /// the rest with bearings only, and `cmap`.
    fn truetype(glyphs: &[Vec<u8>], metrics: &[(u16, i16)], long: u16, cmap: &[u8]) -> Vec<u8> {
        let mut head = vec![0u8; 54];
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[50..52].copy_from_slice(&1u16.to_be_bytes());
        let mut maxp = 0x0001_0000u32.to_be_bytes().to_vec();
        maxp.extend_from_slice(&(glyphs.len() as u16).to_be_bytes());
        maxp.resize(32, 0);
        let mut hhea = vec![0u8; 36];
        hhea[34..36].copy_from_slice(&long.to_be_bytes());
        let mut hmtx = Vec::new();
        for (i, (advance, bearing)) in metrics.iter().enumerate() {
            if i < usize::from(long) {
                hmtx.extend_from_slice(&advance.to_be_bytes());
            }
            hmtx.extend_from_slice(&bearing.to_be_bytes());
        }
        let (glyf, loca) = glyf_loca(glyphs);
        let tables: [([u8; 4], &[u8]); 7] = [
            (*b"cmap", cmap),
            (*b"glyf", &glyf),
            (*b"head", &head),
            (*b"hhea", &hhea),
            (*b"hmtx", &hmtx),
            (*b"loca", &loca),
            (*b"maxp", &maxp),
        ];
        sfnt(*b"\0\x01\0\0", &tables).expect("font")
    }

    /// A simple glyph: one rectangle.
    fn rectangle(x0: i16, y0: i16, x1: i16, y1: i16) -> Vec<u8> {
        let mut glyph = Vec::new();
        for v in [1, x0, y0, x1, y1, 3, 0] {
            glyph.extend_from_slice(&v.to_be_bytes());
        }
        glyph.extend_from_slice(&[1, 1, 1, 1]);
        for v in [x0, x1 - x0, 0, x0 - x1, y0, 0, y1 - y0, 0] {
            glyph.extend_from_slice(&v.to_be_bytes());
        }
        glyph
    }

    /// A composite glyph of components (glyph, flags, arguments, transform
    /// words); MORE_COMPONENTS is set but on the last.
    fn composite(parts: &[(u16, u16, [i16; 2], &[i16])]) -> Vec<u8> {
        let mut glyph = Vec::new();
        for v in [-1i16, 0, 0, 1000, 1000] {
            glyph.extend_from_slice(&v.to_be_bytes());
        }
        for (i, &(gid, flags, args, transform)) in parts.iter().enumerate() {
            let more = if i + 1 < parts.len() { 0x0020 } else { 0 };
            glyph.extend_from_slice(&(flags | more).to_be_bytes());
            glyph.extend_from_slice(&gid.to_be_bytes());
            for arg in args {
                if flags & 0x0001 != 0 {
                    glyph.extend_from_slice(&arg.to_be_bytes());
                } else {
                    glyph.push(arg.to_be_bytes()[1]);
                }
            }
            for v in transform {
                glyph.extend_from_slice(&v.to_be_bytes());
            }
        }
        glyph
    }

    /// `glyphs` as glyf and long loca tables.
    fn glyf_loca(glyphs: &[Vec<u8>]) -> (Vec<u8>, Vec<u8>) {
        let (mut glyf, mut loca) = (Vec::new(), Vec::new());
        for glyph in glyphs {
            loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
            glyf.extend_from_slice(glyph);
        }
        loca.extend_from_slice(&(glyf.len() as u32).to_be_bytes());
        (glyf, loca)
    }

    type Contours = (Vec<(i32, i32)>, Vec<usize>);

    /// A glyph's points and contour ends.
    fn points_of(glyf: &[u8], loca: &[u8], glyph: &[u8]) -> Option<Contours> {
        let mut points = Points::default();
        points.add(glyf, loca, true, glyph, 4, &mut 0)?;
        let points_only = points.points.iter().map(|&(x, y, _)| (x, y)).collect();
        Some((points_only, points.ends))
    }

    /// The UCS2 CMap's character of every CID of `family` (by_cid's view).
    fn collection_chars(family: &CidFamily) -> Vec<Option<char>> {
        let name = family.ucs2_cmap().expect("a UCS2 CMap");
        let ucs2 =
            CMap::parse(load_embedded(name).expect("embedded"), load_embedded).expect("parsed");
        (0..=u32::from(u16::MAX))
            .map(|cid| collection_char(ucs2.lookup_bf_string(cid)))
            .collect()
    }

    /// The glyph the font's format 12 subtable maps `c` to.
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

    /// A glyph's data and long metric in a font `by_cid` built.
    fn glyph_and_metric(font: &Sfnt<'_>, gid: u16) -> (Vec<u8>, (u16, u16)) {
        let data = glyph_data(
            font.table(b"glyf").expect("glyf"),
            font.table(b"loca").expect("loca"),
            true,
            gid,
        );
        let hmtx = font.table(b"hmtx").expect("hmtx");
        let at = 4 * usize::from(gid);
        let metric = (
            u16_at(hmtx, at).expect("advance"),
            u16_at(hmtx, at + 2).expect("bearing"),
        );
        (data.to_vec(), metric)
    }

    #[test]
    fn a_collection_font_puts_the_collections_glyph_at_each_cid() {
        // .notdef; 日, a square; 本, a composite of glyph 3 and of a glyph
        // past the font; glyph 3, a bar no character maps. 中 is missing.
        let square = rectangle(100, 100, 900, 900);
        let glyphs = [
            Vec::new(),
            square.clone(),
            composite(&[(3, 0x0003, [0, 0], &[]), (9, 0x000A, [0, 0], &[0x4000])]),
            rectangle(100, 0, 900, 100),
        ];
        let metrics = [(1000, 0), (900, 100), (0, 100), (0, 20)];
        let pairs = [(0x65E5, 1), (0x672C, 2)];
        let bytes = truetype(&glyphs, &metrics, 2, &cmap4(&pairs));
        let built = by_cid(&bytes, &CidFamily::AdobeGB1).expect("built");
        let font = Sfnt::read(&built).expect("read back");
        let chars = collection_chars(&CidFamily::AdobeGB1);
        let cids = chars.iter().rposition(Option::is_some).expect("CIDs") + 1;
        let cid = |c: char| chars.iter().position(|&x| x == Some(c)).expect("CID") as u16;
        // The CIDs' glyphs only: composites flattened.
        let count = u16_at(font.table(b"maxp").expect("maxp"), 4).expect("count");
        assert_eq!(usize::from(count), cids);
        let hhea = font.table(b"hhea").expect("hhea");
        assert_eq!(u16_at(hhea, 34), Some(count), "every metric long");
        assert_eq!(
            font.table(b"hmtx").map(<[u8]>::len),
            Some(4 * usize::from(count))
        );
        for (at, c) in chars.iter().enumerate().take(cids) {
            if *c == Some('日') {
                let (data, metric) = glyph_and_metric(&font, at as u16);
                assert_eq!(&data[..square.len()], square, "CID {at}");
                assert_eq!(metric, (900, 100));
            }
        }
        let (data, metric) = glyph_and_metric(&font, cid('本'));
        assert_eq!(metric, (900, 100), "a short metric takes the last advance");
        assert_eq!(i16_at(&data, 0), Some(1), "a simple glyph");
        assert_eq!(
            points_of(&[], &[], &data),
            Some((vec![(100, 0), (900, 0), (900, 100), (100, 100)], vec![3])),
            "the bar; the component past the font has no outline"
        );
        for empty in [0, cid('中')] {
            assert!(glyph_and_metric(&font, empty).0.is_empty(), "CID {empty}");
        }
        // Latin from URW Helvetica, at URW's advance.
        let urw = glyphs_of(BuiltinFace::Helvetica);
        let a = urw.iter().find(|g| g.unicode == Some(0x41)).expect("A");
        let (data, metric) = glyph_and_metric(&font, cid('A'));
        assert!(!data.is_empty());
        assert_eq!(metric.0, a.advance);
        let widest = (0..count).map(|gid| glyph_and_metric(&font, gid).1.0).max();
        assert_eq!(u16_at(hhea, 10), widest, "advanceWidthMax");
        assert_eq!(
            [0x65E5, 0x672C, 0x41, 0x4E2D, 0x2_0000].map(|c| map12(&font, c)),
            [Some(cid('日')), Some(cid('本')), Some(cid('A')), None, None]
        );
        for tag in [b"post", b"GSUB", b"vmtx"] {
            assert!(font.table(tag).is_none());
        }
    }

    #[test]
    fn a_collection_font_maps_the_fonts_characters_past_the_bmp() {
        let chars = collection_chars(&CidFamily::AdobeJapan1);
        let (cid, c) = (chars.iter().enumerate())
            .find_map(|(cid, c)| c.filter(|&c| u32::from(c) > 0xFFFF).map(|c| (cid, c)))
            .expect("a Japan1 CID past the BMP");
        // A UCS-4 subtable maps that character to glyph 1.
        let mut cmap: Vec<u8> = [0u16, 1, 3, 10]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        cmap.extend_from_slice(&12u32.to_be_bytes());
        cmap.extend_from_slice(&12u16.to_be_bytes());
        cmap.extend_from_slice(&0u16.to_be_bytes());
        for v in [28, 0, 1, u32::from(c), u32::from(c), 1] {
            cmap.extend_from_slice(&v.to_be_bytes());
        }
        let square = rectangle(100, 100, 900, 900);
        let bytes = truetype(
            &[Vec::new(), square.clone()],
            &[(1000, 0), (1000, 100)],
            2,
            &cmap,
        );
        let built = by_cid(&bytes, &CidFamily::AdobeJapan1).expect("built");
        let font = Sfnt::read(&built).expect("read back");
        let (data, _) = glyph_and_metric(&font, cid as u16);
        assert_eq!(&data[..square.len()], square);
        assert_eq!(map12(&font, u32::from(c)), Some(cid as u16));
    }

    #[test]
    fn a_collection_font_stays_within_its_budget() {
        // Every CJK unified ideograph maps to glyph 1 (a glyph id array).
        let (first, last) = (0x4E00u16, 0x9FA5u16);
        let mut sub = Vec::new();
        let n = usize::from(last - first) + 1;
        for v in [4u16, (32 + 2 * n) as u16, 0, 4, 4, 1, 0, last, 0xFFFF, 0] {
            sub.extend_from_slice(&v.to_be_bytes());
        }
        for v in [first, 0xFFFF, 0, 1, 4, 0] {
            sub.extend_from_slice(&v.to_be_bytes());
        }
        sub.extend((0..n).flat_map(|_| 1u16.to_be_bytes()));
        let mut cmap: Vec<u8> = [0u16, 1, 3, 1]
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect();
        cmap.extend_from_slice(&12u32.to_be_bytes());
        cmap.extend_from_slice(&sub);
        let font = |glyph: Vec<u8>| truetype(&[Vec::new(), glyph], &[(1000, 0); 2], 2, &cmap);
        // A copy per CID of a 64 KiB glyph is past the budget...
        let mut large = rectangle(0, 0, 1000, 1000);
        large.resize(1 << 16, 0);
        assert!(by_cid(&font(large), &CidFamily::AdobeGB1).is_none());
        // ...of a small one within it.
        let small = rectangle(0, 0, 1000, 1000);
        let built = by_cid(&font(small.clone()), &CidFamily::AdobeGB1).expect("built");
        let built = Sfnt::read(&built).expect("read back");
        let chars = collection_chars(&CidFamily::AdobeGB1);
        let cid = chars.iter().position(|&c| c == Some('中')).expect("CID");
        let (data, _) = glyph_and_metric(&built, cid as u16);
        assert_eq!(&data[..small.len()], small);
    }

    #[test]
    fn composite_glyphs_flatten_with_their_offsets_and_transforms() {
        let square = rectangle(0, 0, 100, 100);
        // 2: the square at (10, 20) (words), then halved at (-5, 7) (bytes).
        // 3: glyph 2 with x and y swapped (2x2), then the square with its
        // point 0 on point 1 (point numbers). 4 refers to itself.
        let two = composite(&[(1, 0x0003, [10, 20], &[]), (1, 0x000A, [-5, 7], &[0x2000])]);
        let swap: &[i16] = &[0, 0x4000, 0x4000, 0];
        let three = composite(&[(2, 0x0082, [0, 0], swap), (1, 0x0001, [1, 0], &[])]);
        let four = composite(&[(4, 0x0002, [0, 0], &[])]);
        let (glyf, loca) = glyf_loca(&[Vec::new(), square, two, three, four]);
        let flat = |g: u16| points_of(&glyf, &loca, glyph_data(&glyf, &loca, true, g));
        let (points, ends) = flat(2).expect("glyph 2");
        assert_eq!(
            points,
            [
                (10, 20),
                (110, 20),
                (110, 120),
                (10, 120),
                (-5, 7),
                (45, 7),
                (45, 57),
                (-5, 57)
            ]
        );
        assert_eq!(ends, [3, 7]);
        let (points, ends) = flat(3).expect("glyph 3");
        assert_eq!(
            points,
            [
                (20, 10),
                (20, 110),
                (120, 110),
                (120, 10),
                (7, -5),
                (7, 45),
                (57, 45),
                (57, -5),
                (20, 110),
                (120, 110),
                (120, 210),
                (20, 210)
            ]
        );
        assert_eq!(ends, [3, 7, 11]);
        assert!(flat(4).is_none(), "a cycle stops at the depth");
        let mut work = MAX_FLATTEN_WORK;
        let data = glyph_data(&glyf, &loca, true, 2);
        assert!(
            Points::default()
                .add(&glyf, &loca, true, data, 4, &mut work)
                .is_none()
        );
        // Written as a simple glyph and read back, deltas of a byte, none
        // and a word.
        let mut outline = Points::default();
        (outline.add(
            &glyf,
            &loca,
            true,
            glyph_data(&glyf, &loca, true, 3),
            4,
            &mut 0,
        ))
        .expect("glyph 3");
        outline.points.push((-1000, 3000, false));
        outline.ends.push(12);
        let mut bytes = Vec::new();
        assert_eq!(outline.write(&mut bytes), Some((13, 4)));
        let (back, ends) = points_of(&[], &[], &bytes).expect("read back");
        assert_eq!(back.len(), 13);
        assert_eq!((back[8], back[12]), ((20, 110), (-1000, 3000)));
        assert_eq!(ends, [3, 7, 11, 12]);
        let written = bytes.len();
        outline.clear();
        assert_eq!(outline.write(&mut bytes), Some((0, 0)));
        assert_eq!(bytes.len(), written, "no contours, no data");
    }

    #[test]
    fn a_cid_draws_the_base_of_a_variation_sequence_and_no_other_sequence() {
        let string = |s: &str| Some(BfString::String(s.into()));
        assert_eq!(collection_char(Some(BfString::Char('日'))), Some('日'));
        assert_eq!(collection_char(string("郎\u{FE00}")), Some('郎'));
        assert_eq!(collection_char(string("葛\u{E0100}")), Some('葛'));
        assert_eq!(collection_char(string("3/10")), None);
        assert_eq!(collection_char(string("\u{31F7}\u{309A}")), None);
        assert_eq!(collection_char(None), None);
    }

    #[test]
    fn only_a_truetype_font_is_rebuilt() {
        let program = Face::Helvetica.program().expect("an OpenType CFF font");
        assert!(by_cid((*program).as_ref(), &CidFamily::AdobeGB1).is_none());
        let square = rectangle(100, 100, 900, 900);
        let bytes = truetype(
            &[Vec::new(), square],
            &[(1000, 0); 2],
            2,
            &cmap4(&[(0x65E5, 1)]),
        );
        assert!(by_cid(&bytes, &CidFamily::AdobeIdentity).is_none());
        assert!(by_cid(&bytes, &CidFamily::AdobeKorea1).is_some());
    }
}
