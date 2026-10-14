//! Bounded unhinted simple TrueType outlines selected by an explicit glyph ID.
//!
//! No cmap selection, metrics, hint execution, composite expansion or rendering.
//! Table checksums are retained but not verified. Format interpretation follows
//! OpenType glyf/loca/head/maxp; exact contour start/closure follows the documented
//! FreeType-compatible decomposition convention. See TRUETYPE-OUTLINE-DESIGN.md.
// Contour canonicalization is informed by FreeType ftoutln.c, Copyright
// (C) 1996-2026 David Turner, Robert Wilhelm, and Werner Lemberg.
// Its notice and FreeType License are retained in reference/licenses/freetype-native/.
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::mem::size_of;
use std::ops::Range;

const MAX_COMPONENT_DEPTH: usize = 16;
const COMPOSITE_COORDINATE_BOUND: i32 = 1 << 20;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontLimits {
    pub max_input_bytes: usize,
    pub max_tables: usize,
    pub max_glyphs: usize,
    pub max_index_allocation_bytes: usize,
    pub max_work: usize,
}
impl Default for FontLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 32 << 20,
            max_tables: 256,
            max_glyphs: 65_535,
            max_index_allocation_bytes: 2 << 20,
            max_work: 4 << 20,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FontUsage {
    pub index_allocation_bytes: usize,
    pub work: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutlineLimits {
    pub max_decode_attempts: usize,
    pub max_contours_per_glyph: usize,
    pub max_points_per_glyph: usize,
    pub max_instruction_bytes_per_glyph: usize,
    pub max_total_glyph_bytes: usize,
    pub max_total_contours: usize,
    pub max_total_points: usize,
    pub max_total_commands: usize,
    pub max_allocation_bytes: usize,
    pub max_work: usize,
}
impl Default for OutlineLimits {
    fn default() -> Self {
        Self {
            max_decode_attempts: 65_536,
            max_contours_per_glyph: 4096,
            max_points_per_glyph: 32_768,
            max_instruction_bytes_per_glyph: 65_535,
            max_total_glyph_bytes: 64 << 20,
            max_total_contours: 262_144,
            max_total_points: 1_048_576,
            max_total_commands: 2_097_152,
            max_allocation_bytes: 64 << 20,
            max_work: 16_777_216,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutlineUsage {
    pub decode_attempts: usize,
    pub glyph_bytes: usize,
    pub contours: usize,
    pub points: usize,
    pub commands: usize,
    pub allocation_bytes: usize,
    pub work: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineErrorKind {
    Malformed(&'static str),
    Unsupported(&'static str),
    Limit(&'static str),
    Allocation,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineError {
    pub offset: usize,
    pub table: Option<[u8; 4]>,
    pub glyph: Option<u16>,
    pub kind: OutlineErrorKind,
}
impl std::fmt::Display for OutlineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TrueType outline error at byte {}", self.offset)?;
        if let Some(t) = self.table {
            write!(f, " in {}", String::from_utf8_lossy(&t))?;
        }
        if let Some(g) = self.glyph {
            write!(f, " for glyph {g}")?;
        }
        write!(f, ": {:?}", self.kind)
    }
}
impl std::error::Error for OutlineError {}
fn error(offset: usize, kind: OutlineErrorKind) -> OutlineError {
    OutlineError {
        offset,
        table: None,
        glyph: None,
        kind,
    }
}
fn malformed(offset: usize, why: &'static str) -> OutlineError {
    error(offset, OutlineErrorKind::Malformed(why))
}
fn charge(
    value: &mut usize,
    amount: usize,
    max: usize,
    label: &'static str,
    offset: usize,
) -> Result<(), OutlineError> {
    let next = value
        .checked_add(amount)
        .ok_or_else(|| error(offset, OutlineErrorKind::Limit(label)))?;
    if next > max {
        return Err(error(offset, OutlineErrorKind::Limit(label)));
    }
    *value = next;
    Ok(())
}
fn bytes_for<T>(count: usize, offset: usize) -> Result<usize, OutlineError> {
    count
        .checked_mul(size_of::<T>())
        .ok_or_else(|| error(offset, OutlineErrorKind::Limit("allocation size overflow")))
}
fn reserve<T>(v: &mut Vec<T>, count: usize, offset: usize) -> Result<(), OutlineError> {
    v.try_reserve_exact(count)
        .map_err(|_| error(offset, OutlineErrorKind::Allocation))
}
fn checked_slice(bytes: &[u8], offset: usize, length: usize) -> Result<&[u8], OutlineError> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| malformed(offset, "byte range overflow"))?;
    bytes
        .get(offset..end)
        .ok_or_else(|| malformed(offset, "truncated byte range"))
}
fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, OutlineError> {
    let s = checked_slice(bytes, offset, 2)?;
    Ok(u16::from_be_bytes([s[0], s[1]]))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, OutlineError> {
    let s = checked_slice(bytes, offset, 4)?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}
fn padding(bytes: &[u8], offset: usize) -> Result<(), OutlineError> {
    if bytes.len() > 3 || bytes.iter().any(|b| *b != 0) {
        return Err(malformed(offset, "nonzero or excessive trailing padding"));
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRecord {
    tag: [u8; 4],
    checksum: u32,
    range: Range<usize>,
}
impl TableRecord {
    pub fn tag(&self) -> [u8; 4] {
        self.tag
    }
    pub fn checksum(&self) -> u32 {
        self.checksum
    }
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }
    pub fn offset(&self) -> usize {
        self.range.start
    }
    pub fn length(&self) -> usize {
        self.range.len()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphId(u16);
impl GlyphId {
    pub fn get(self) -> u16 {
        self.0
    }
    /// Unchecked against any font; decoding still range-checks the ID.
    pub(crate) fn from_raw(id: u16) -> Self {
        Self(id)
    }
}
#[derive(Debug)]
pub struct TrueTypeFont<'a> {
    bytes: &'a [u8],
    tables: Vec<TableRecord>,
    loca: Vec<u32>,
    glyf: Range<usize>,
    units: u16,
    glyphs: u16,
    max_points: u16,
    max_contours: u16,
    usage: FontUsage,
}
impl<'a> TrueTypeFont<'a> {
    pub fn open(bytes: &'a [u8], limits: FontLimits) -> Result<Self, OutlineError> {
        let mut usage = FontUsage::default();
        if bytes.len() > limits.max_input_bytes {
            return Err(error(0, OutlineErrorKind::Limit("font input bytes")));
        }
        if u32_at(bytes, 0)? != 0x0001_0000 {
            return Err(error(
                0,
                OutlineErrorKind::Unsupported("sfnt version/container"),
            ));
        }
        let count = usize::from(u16_at(bytes, 4)?);
        if count == 0 {
            return Err(malformed(4, "empty table directory"));
        }
        if count > limits.max_tables {
            return Err(error(4, OutlineErrorKind::Limit("table count")));
        }
        let directory_end = count
            .checked_mul(16)
            .and_then(|n| n.checked_add(12))
            .ok_or_else(|| malformed(4, "directory size overflow"))?;
        checked_slice(bytes, 0, directory_end)?;
        let sort_work = count
            .checked_mul(2 * (usize::BITS - count.leading_zeros()) as usize + 1)
            .ok_or_else(|| malformed(4, "directory work overflow"))?;
        charge(
            &mut usage.work,
            directory_end,
            limits.max_work,
            "font work",
            0,
        )?;
        charge(&mut usage.work, sort_work, limits.max_work, "font work", 0)?;
        charge(
            &mut usage.index_allocation_bytes,
            bytes_for::<TableRecord>(count, 4)?,
            limits.max_index_allocation_bytes,
            "font index allocation",
            4,
        )?;
        let mut tables = Vec::new();
        reserve(&mut tables, count, 4)?;
        for i in 0..count {
            let p = 12 + 16 * i;
            let tag = [bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]];
            let offset = usize::try_from(u32_at(bytes, p + 8)?)
                .map_err(|_| malformed(p + 8, "table offset overflow"))?;
            let length = usize::try_from(u32_at(bytes, p + 12)?)
                .map_err(|_| malformed(p + 12, "table length overflow"))?;
            checked_slice(bytes, offset, length).map_err(|mut e| {
                e.table = Some(tag);
                e
            })?;
            if length != 0 && offset < directory_end {
                return Err(malformed(p + 8, "table overlaps directory"));
            }
            tables.push(TableRecord {
                tag,
                checksum: u32_at(bytes, p + 4)?,
                range: offset..offset + length,
            });
        }
        tables.sort_unstable_by_key(|t| (t.range.start, t.range.end));
        let mut previous_end = directory_end;
        for table in &tables {
            if table.range.is_empty() {
                continue;
            }
            if table.range.start < previous_end {
                return Err(malformed(table.range.start, "overlapping tables"));
            }
            previous_end = table.range.end;
        }
        tables.sort_unstable_by_key(|t| t.tag);
        if tables.windows(2).any(|w| w[0].tag == w[1].tag) {
            return Err(malformed(12, "duplicate table tag"));
        }
        for tag in [*b"fvar", *b"gvar", *b"avar", *b"HVAR", *b"VVAR", *b"MVAR"] {
            if let Ok(i) = tables.binary_search_by_key(&tag, |t| t.tag) {
                return Err(error(
                    tables[i].range.start,
                    OutlineErrorKind::Unsupported("variation table"),
                ));
            }
        }
        let find = |tag: [u8; 4]| -> Result<Range<usize>, OutlineError> {
            tables
                .binary_search_by_key(&tag, |t| t.tag)
                .map(|i| tables[i].range.clone())
                .map_err(|_| {
                    let mut e = malformed(12, "missing required outline table");
                    e.table = Some(tag);
                    e
                })
        };
        let head = find(*b"head")?;
        let maxp = find(*b"maxp")?;
        let loca_range = find(*b"loca")?;
        let glyf = find(*b"glyf")?;
        let parse_metadata = || -> Result<(u16, u16, u16, u16, u16), OutlineError> {
            let h = checked_slice(bytes, head.start, head.len())?;
            checked_slice(h, 0, 54).map_err(|mut e| {
                e.offset += head.start;
                e
            })?;
            padding(&h[54..], head.start + 54)?;
            if u32_at(h, 0)? != 0x0001_0000 || u32_at(h, 12)? != 0x5f0f_3cf5 {
                return Err(malformed(head.start, "head version or magic"));
            }
            let units = u16_at(h, 18)?;
            if !(16..=16_384).contains(&units) {
                return Err(malformed(head.start + 18, "unitsPerEm outside 16..16384"));
            }
            let format = u16_at(h, 50)?;
            if format > 1 || u16_at(h, 52)? != 0 {
                return Err(error(
                    head.start + 50,
                    OutlineErrorKind::Unsupported("loca/glyph data format"),
                ));
            }
            let m = checked_slice(bytes, maxp.start, maxp.len())?;
            checked_slice(m, 0, 32).map_err(|mut e| {
                e.offset += maxp.start;
                e
            })?;
            padding(&m[32..], maxp.start + 32)?;
            if u32_at(m, 0)? != 0x0001_0000 {
                return Err(error(
                    maxp.start,
                    OutlineErrorKind::Unsupported("maxp version"),
                ));
            }
            let glyphs = u16_at(m, 4)?;
            if glyphs == 0 {
                return Err(malformed(maxp.start + 4, "zero glyph count"));
            }
            Ok((units, format, glyphs, u16_at(m, 6)?, u16_at(m, 8)?))
        };
        charge(
            &mut usage.work,
            head.len()
                .checked_add(maxp.len())
                .ok_or_else(|| malformed(head.start, "metadata size overflow"))?,
            limits.max_work,
            "font work",
            head.start,
        )?;
        let (units, format, glyphs, max_points, max_contours) = parse_metadata()?;
        if usize::from(glyphs) > limits.max_glyphs {
            return Err(error(
                maxp.start + 4,
                OutlineErrorKind::Limit("glyph count"),
            ));
        }
        let entries = usize::from(glyphs) + 1;
        let width = if format == 0 { 2 } else { 4 };
        let expected = entries * width;
        let l = checked_slice(bytes, loca_range.start, loca_range.len())?;
        checked_slice(l, 0, expected).map_err(|mut e| {
            e.offset += loca_range.start;
            e
        })?;
        charge(
            &mut usage.work,
            loca_range.len(),
            limits.max_work,
            "font work",
            loca_range.start,
        )?;
        padding(&l[expected..], loca_range.start + expected)?;
        charge(
            &mut usage.index_allocation_bytes,
            bytes_for::<u32>(entries, loca_range.start)?,
            limits.max_index_allocation_bytes,
            "font index allocation",
            loca_range.start,
        )?;
        let mut loca = Vec::new();
        reserve(&mut loca, entries, loca_range.start)?;
        for i in 0..entries {
            let offset = if width == 2 {
                u32::from(u16_at(l, i * 2)?) * 2
            } else {
                u32_at(l, i * 4)?
            };
            if (i != 0 && offset < loca[i - 1]) || u64::from(offset) > glyf.len() as u64 {
                return Err(malformed(
                    loca_range.start + i * width,
                    "nonmonotone or out-of-bounds loca",
                ));
            }
            loca.push(offset);
        }
        Ok(Self {
            bytes,
            tables,
            loca,
            glyf,
            units,
            glyphs,
            max_points,
            max_contours,
            usage,
        })
    }
    pub fn units_per_em(&self) -> u16 {
        self.units
    }
    pub fn glyph_count(&self) -> u16 {
        self.glyphs
    }
    /// Internal immutable access through the validated sfnt directory.
    pub(crate) fn table_data(&self, tag: [u8; 4]) -> Option<&'a [u8]> {
        self.tables
            .iter()
            .find(|record| record.tag == tag)
            .map(|record| &self.bytes[record.range.clone()])
    }
    pub fn table_records(&self) -> &[TableRecord] {
        &self.tables
    }
    pub fn usage(&self) -> FontUsage {
        self.usage
    }
    pub fn glyph_id(&self, raw: u32) -> Result<GlyphId, OutlineError> {
        if raw >= u32::from(self.glyphs) {
            return Err(malformed(0, "glyph ID out of range"));
        }
        Ok(GlyphId(raw as u16))
    }
    pub fn session(&self, limits: OutlineLimits) -> OutlineSession<'_, 'a> {
        OutlineSession {
            font: self,
            limits,
            usage: OutlineUsage::default(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontPoint {
    x: i32,
    y: i32,
    on_curve: bool,
}
impl FontPoint {
    pub fn x(self) -> i32 {
        self.x
    }
    pub fn y(self) -> i32 {
        self.y
    }
    pub fn on_curve(self) -> bool {
        self.on_curve
    }
    fn doubled(self) -> FontPoint2 {
        FontPoint2 {
            x2: self.x * 2,
            y2: self.y * 2,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontPoint2 {
    x2: i32,
    y2: i32,
}
impl FontPoint2 {
    pub fn x2(self) -> i32 {
        self.x2
    }
    pub fn y2(self) -> i32 {
        self.y2
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphBounds {
    x_min: i32,
    y_min: i32,
    x_max: i32,
    y_max: i32,
}
impl GlyphBounds {
    pub fn coordinates(self) -> [i32; 4] {
        [self.x_min, self.y_min, self.x_max, self.y_max]
    }
    pub fn x_min(self) -> i32 {
        self.x_min
    }
    pub fn y_min(self) -> i32 {
        self.y_min
    }
    pub fn x_max(self) -> i32 {
        self.x_max
    }
    pub fn y_max(self) -> i32 {
        self.y_max
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineCommand {
    MoveTo(FontPoint2),
    LineTo(FontPoint2),
    QuadTo {
        control: FontPoint2,
        end: FontPoint2,
    },
    Close,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineKind {
    EmptyLocaRange,
    ZeroContours,
    Simple,
    /// Components flattened into one point list; transformed coordinates are
    /// rounded to font units like FreeType's unscaled load.
    Composite,
}
#[derive(Debug, PartialEq, Eq)]
pub struct GlyphOutline {
    id: GlyphId,
    kind: OutlineKind,
    range: Range<usize>,
    points: Vec<FontPoint>,
    endpoints: Vec<u16>,
    commands: Vec<OutlineCommand>,
    header_bounds: Option<GlyphBounds>,
    control_bounds: Option<GlyphBounds>,
    instruction_bytes: usize,
    overlap: bool,
}
impl GlyphOutline {
    pub fn id(&self) -> GlyphId {
        self.id
    }
    pub fn kind(&self) -> OutlineKind {
        self.kind
    }
    pub fn glyph_range(&self) -> Range<usize> {
        self.range.clone()
    }
    pub fn points(&self) -> &[FontPoint] {
        &self.points
    }
    pub fn contour_endpoints(&self) -> &[u16] {
        &self.endpoints
    }
    pub fn commands(&self) -> &[OutlineCommand] {
        &self.commands
    }
    pub fn header_bounds(&self) -> Option<GlyphBounds> {
        self.header_bounds
    }
    pub fn control_bounds(&self) -> Option<GlyphBounds> {
        self.control_bounds
    }
    pub fn instruction_bytes(&self) -> usize {
        self.instruction_bytes
    }
    pub fn overlap(&self) -> bool {
        self.overlap
    }
}
#[derive(Debug)]
pub struct OutlineSession<'font, 'bytes> {
    font: &'font TrueTypeFont<'bytes>,
    limits: OutlineLimits,
    usage: OutlineUsage,
}
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
    base: usize,
}
impl Cursor<'_> {
    fn offset(&self) -> usize {
        self.base + self.at
    }
    fn take(&mut self, n: usize) -> Result<&[u8], OutlineError> {
        let s = checked_slice(self.bytes, self.at, n).map_err(|mut e| {
            e.offset += self.base;
            e
        })?;
        self.at += n;
        Ok(s)
    }
    fn byte(&mut self) -> Result<u8, OutlineError> {
        Ok(self.take(1)?[0])
    }
    fn word(&mut self) -> Result<u16, OutlineError> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    fn signed(&mut self) -> Result<i16, OutlineError> {
        Ok(self.word()? as i16)
    }
}
impl OutlineSession<'_, '_> {
    pub fn usage(&self) -> OutlineUsage {
        self.usage
    }
    fn work(&mut self, n: usize, offset: usize) -> Result<(), OutlineError> {
        charge(
            &mut self.usage.work,
            n,
            self.limits.max_work,
            "outline work",
            offset,
        )
    }
    fn allocate<T>(&mut self, n: usize, offset: usize) -> Result<Vec<T>, OutlineError> {
        charge(
            &mut self.usage.allocation_bytes,
            bytes_for::<T>(n, offset)?,
            self.limits.max_allocation_bytes,
            "outline allocation",
            offset,
        )?;
        let mut v = Vec::new();
        reserve(&mut v, n, offset)?;
        Ok(v)
    }
    pub fn decode(&mut self, id: GlyphId) -> Result<GlyphOutline, OutlineError> {
        self.decode_inner(id).map_err(|mut e| {
            e.table = Some(*b"glyf");
            e.glyph = Some(id.0);
            e
        })
    }
    fn decode_inner(&mut self, id: GlyphId) -> Result<GlyphOutline, OutlineError> {
        let mut result = self.parse(id, 0)?;
        if result.kind != OutlineKind::Simple && result.kind != OutlineKind::Composite {
            return Ok(result);
        }
        let start = result.range.start;
        let count = result.points.len();
        let contours = result.endpoints.len();
        // Each point causes at most one line/quad; each contour needs move, closure, Close.
        let max_commands = count
            .checked_add(
                contours
                    .checked_mul(3)
                    .ok_or_else(|| malformed(start, "command bound overflow"))?,
            )
            .ok_or_else(|| malformed(start, "command bound overflow"))?;
        let remaining_commands = self
            .limits
            .max_total_commands
            .saturating_sub(self.usage.commands);
        let mut commands =
            self.allocate::<OutlineCommand>(max_commands.min(remaining_commands), start)?;
        let mut first = 0;
        for &last in &result.endpoints {
            let end = usize::from(last) + 1;
            self.contour(&result.points[first..end], &mut commands, start)?;
            first = end;
        }
        result.commands = commands;
        Ok(result)
    }
    /// Glyph points and contour endpoints without commands; composites recurse.
    fn parse(&mut self, id: GlyphId, depth: usize) -> Result<GlyphOutline, OutlineError> {
        charge(
            &mut self.usage.decode_attempts,
            1,
            self.limits.max_decode_attempts,
            "glyph decode attempts",
            0,
        )?;
        self.work(1, 0)?;
        if id.0 >= self.font.glyphs {
            return Err(malformed(0, "glyph ID out of range"));
        }
        let i = usize::from(id.0);
        let range = self.font.glyf.start + self.font.loca[i] as usize
            ..self.font.glyf.start + self.font.loca[i + 1] as usize;
        charge(
            &mut self.usage.glyph_bytes,
            range.len(),
            self.limits.max_total_glyph_bytes,
            "total glyph bytes",
            range.start,
        )?;
        self.work(range.len(), range.start)?;
        let mut result = GlyphOutline {
            id,
            kind: OutlineKind::EmptyLocaRange,
            range: range.clone(),
            points: Vec::new(),
            endpoints: Vec::new(),
            commands: Vec::new(),
            header_bounds: None,
            control_bounds: None,
            instruction_bytes: 0,
            overlap: false,
        };
        if range.is_empty() {
            return Ok(result);
        }
        let mut c = Cursor {
            bytes: &self.font.bytes[range.clone()],
            at: 0,
            base: range.start,
        };
        let contours = c.signed()?;
        let bounds = GlyphBounds {
            x_min: i32::from(c.signed()?),
            y_min: i32::from(c.signed()?),
            x_max: i32::from(c.signed()?),
            y_max: i32::from(c.signed()?),
        };
        if contours < 0 {
            result.header_bounds = Some(bounds);
            return self.composite(result, c, depth);
        }
        if bounds.x_min > bounds.x_max || bounds.y_min > bounds.y_max {
            return Err(malformed(range.start + 2, "reversed glyph bounds"));
        }
        result.header_bounds = Some(bounds);
        let contours = contours as usize;
        if contours > self.limits.max_contours_per_glyph {
            return Err(error(
                range.start,
                OutlineErrorKind::Limit("contours per glyph"),
            ));
        }
        if contours > usize::from(self.font.max_contours) {
            return Err(malformed(range.start, "contours exceed maxp"));
        }
        charge(
            &mut self.usage.contours,
            contours,
            self.limits.max_total_contours,
            "total contours",
            range.start,
        )?;
        self.work(contours, range.start)?;
        let mut endpoints = self.allocate::<u16>(contours, c.offset())?;
        for _ in 0..contours {
            let offset = c.offset();
            let end = c.word()?;
            if endpoints.last().is_some_and(|prior| *prior >= end) {
                return Err(malformed(offset, "nonincreasing contour endpoints"));
            }
            endpoints.push(end);
        }
        let count = endpoints.last().map_or(0, |v| usize::from(*v) + 1);
        if count > self.limits.max_points_per_glyph {
            return Err(error(
                c.offset(),
                OutlineErrorKind::Limit("points per glyph"),
            ));
        }
        if count > usize::from(self.font.max_points) {
            return Err(malformed(c.offset(), "points exceed maxp"));
        }
        charge(
            &mut self.usage.points,
            count,
            self.limits.max_total_points,
            "total points",
            c.offset(),
        )?;
        // Zero-contour glyphs may consist solely of their header, plus one pad byte.
        if contours == 0 && c.bytes.len() - c.at <= 1 {
            padding(&c.bytes[c.at..], c.offset())?;
            result.kind = OutlineKind::ZeroContours;
            return Ok(result);
        }
        let instruction_count = usize::from(c.word()?);
        if instruction_count > self.limits.max_instruction_bytes_per_glyph {
            return Err(error(
                c.offset() - 2,
                OutlineErrorKind::Limit("glyph instruction bytes"),
            ));
        }
        c.take(instruction_count)?;
        result.instruction_bytes = instruction_count;
        let mut flags = self.allocate::<u8>(count, c.offset())?;
        while flags.len() < count {
            let offset = c.offset();
            let flag = c.byte()?;
            if flag & 0x80 != 0 {
                return Err(malformed(offset, "reserved glyph flag"));
            }
            if flag & 0x40 != 0 && !flags.is_empty() && flags[0] & 0x40 == 0 {
                return Err(malformed(offset, "overlap flag absent on first point"));
            }
            let repeats = if flag & 8 != 0 {
                usize::from(c.byte()?) + 1
            } else {
                1
            };
            if repeats > count - flags.len() {
                return Err(malformed(offset, "flag repeat exceeds point count"));
            }
            self.work(repeats, offset)?;
            flags.resize(flags.len() + repeats, flag);
        }
        let mut points = self.allocate::<FontPoint>(count, c.offset())?;
        let mut x = 0i32;
        for &flag in &flags {
            self.work(1, c.offset())?;
            x = coordinate(&mut c, flag, 2, 16, x)?;
            points.push(FontPoint {
                x,
                y: 0,
                on_curve: flag & 1 != 0,
            });
        }
        let mut y = 0i32;
        for (&flag, point) in flags.iter().zip(&mut points) {
            self.work(1, c.offset())?;
            y = coordinate(&mut c, flag, 4, 32, y)?;
            point.y = y;
        }
        padding(&c.bytes[c.at..], c.offset())?;
        self.work(count, c.offset())?;
        let control = points.first().map(|p| {
            let mut b = GlyphBounds {
                x_min: p.x,
                y_min: p.y,
                x_max: p.x,
                y_max: p.y,
            };
            for p in &points {
                b.x_min = b.x_min.min(p.x);
                b.y_min = b.y_min.min(p.y);
                b.x_max = b.x_max.max(p.x);
                b.y_max = b.y_max.max(p.y);
            }
            b
        });
        if let Some(actual) = control {
            if actual != bounds {
                return Err(malformed(
                    range.start + 2,
                    "glyph bounds differ from control points",
                ));
            }
        }
        result.kind = if contours == 0 {
            OutlineKind::ZeroContours
        } else {
            OutlineKind::Simple
        };
        result.overlap = flags.first().is_some_and(|f| f & 0x40 != 0);
        result.points = points;
        result.endpoints = endpoints;
        result.control_bounds = control;
        Ok(result)
    }
    /// OpenType composite glyph: each component's points are transformed by
    /// its F2Dot14 matrix, rounded, then offset. Anchor-point matching is
    /// refused. Hinting instructions are skipped (unhinted outlines).
    fn composite(
        &mut self,
        mut result: GlyphOutline,
        mut c: Cursor<'_>,
        depth: usize,
    ) -> Result<GlyphOutline, OutlineError> {
        const ARG_WORDS: u16 = 0x1;
        const XY_VALUES: u16 = 0x2;
        const SCALE: u16 = 0x8;
        const MORE: u16 = 0x20;
        const XY_SCALE: u16 = 0x40;
        const TWO_BY_TWO: u16 = 0x80;
        const SCALED_OFFSET: u16 = 0x800;
        if depth >= MAX_COMPONENT_DEPTH {
            return Err(error(
                c.offset(),
                OutlineErrorKind::Limit("composite depth"),
            ));
        }
        let mut points: Vec<FontPoint> = Vec::new();
        let mut endpoints: Vec<u16> = Vec::new();
        loop {
            let offset = c.offset();
            self.work(1, offset)?;
            let flags = c.word()?;
            let glyph = c.word()?;
            let (dx, dy) = if flags & ARG_WORDS != 0 {
                (i32::from(c.signed()?), i32::from(c.signed()?))
            } else {
                (i32::from(c.byte()? as i8), i32::from(c.byte()? as i8))
            };
            if flags & XY_VALUES == 0 {
                return Err(error(
                    offset,
                    OutlineErrorKind::Unsupported("composite anchor-point matching"),
                ));
            }
            let f2 = |c: &mut Cursor<'_>| -> Result<f64, OutlineError> {
                Ok(f64::from(c.signed()?) / 16384.)
            };
            let matrix = if flags & SCALE != 0 {
                let v = f2(&mut c)?;
                Some([v, 0., 0., v])
            } else if flags & XY_SCALE != 0 {
                Some([f2(&mut c)?, 0., 0., f2(&mut c)?])
            } else if flags & TWO_BY_TWO != 0 {
                Some([f2(&mut c)?, f2(&mut c)?, f2(&mut c)?, f2(&mut c)?])
            } else {
                None
            };
            if glyph >= self.font.glyphs {
                return Err(malformed(offset, "component glyph ID out of range"));
            }
            let component = self.parse(GlyphId(glyph), depth + 1)?;
            let base = points.len();
            let total = base + component.points.len();
            if total > self.limits.max_points_per_glyph {
                return Err(error(offset, OutlineErrorKind::Limit("points per glyph")));
            }
            if endpoints.len() + component.endpoints.len() > self.limits.max_contours_per_glyph {
                return Err(error(offset, OutlineErrorKind::Limit("contours per glyph")));
            }
            reserve(&mut points, component.points.len(), offset)?;
            reserve(&mut endpoints, component.endpoints.len(), offset)?;
            let transform = |x: i32, y: i32| -> (f64, f64) {
                match matrix {
                    // x' = xscale*x + scale10*y; y' = scale01*x + yscale*y
                    Some([a, b, c, d]) => (
                        a * f64::from(x) + c * f64::from(y),
                        b * f64::from(x) + d * f64::from(y),
                    ),
                    None => (f64::from(x), f64::from(y)),
                }
            };
            let (ox, oy) = if flags & SCALED_OFFSET != 0 {
                transform(dx, dy)
            } else {
                (f64::from(dx), f64::from(dy))
            };
            for p in &component.points {
                self.work(1, offset)?;
                let (x, y) = transform(p.x, p.y);
                let (x, y) = ((x.round() + ox.round()), (y.round() + oy.round()));
                if x.abs() > f64::from(COMPOSITE_COORDINATE_BOUND)
                    || y.abs() > f64::from(COMPOSITE_COORDINATE_BOUND)
                {
                    return Err(malformed(offset, "composite coordinate out of range"));
                }
                points.push(FontPoint {
                    x: x as i32,
                    y: y as i32,
                    on_curve: p.on_curve,
                });
            }
            for &end in &component.endpoints {
                endpoints.push(
                    u16::try_from(base + usize::from(end))
                        .map_err(|_| malformed(offset, "composite point index"))?,
                );
            }
            result.overlap |= component.overlap;
            if flags & MORE == 0 {
                break;
            }
        }
        // Remaining bytes are optional instructions and padding.
        charge(
            &mut self.usage.allocation_bytes,
            bytes_for::<FontPoint>(points.capacity(), c.offset())?
                .saturating_add(bytes_for::<u16>(endpoints.capacity(), c.offset())?),
            self.limits.max_allocation_bytes,
            "outline allocation",
            c.offset(),
        )?;
        result.control_bounds = points.first().map(|p| {
            points.iter().fold(
                GlyphBounds {
                    x_min: p.x,
                    y_min: p.y,
                    x_max: p.x,
                    y_max: p.y,
                },
                |b, p| GlyphBounds {
                    x_min: b.x_min.min(p.x),
                    y_min: b.y_min.min(p.y),
                    x_max: b.x_max.max(p.x),
                    y_max: b.y_max.max(p.y),
                },
            )
        });
        result.kind = if endpoints.is_empty() {
            OutlineKind::ZeroContours
        } else {
            OutlineKind::Composite
        };
        result.points = points;
        result.endpoints = endpoints;
        Ok(result)
    }
    fn emit(
        &mut self,
        out: &mut Vec<OutlineCommand>,
        command: OutlineCommand,
        offset: usize,
    ) -> Result<(), OutlineError> {
        charge(
            &mut self.usage.commands,
            1,
            self.limits.max_total_commands,
            "total outline commands",
            offset,
        )?;
        self.work(1, offset)?;
        if out.len() == out.capacity() {
            return Err(malformed(offset, "internal command bound exceeded"));
        }
        out.push(command);
        Ok(())
    }
    fn contour(
        &mut self,
        p: &[FontPoint],
        out: &mut Vec<OutlineCommand>,
        offset: usize,
    ) -> Result<(), OutlineError> {
        let first = p[0];
        let last = p[p.len() - 1];
        let (start, mut i, end) = if first.on_curve {
            (first.doubled(), 1, p.len())
        } else if last.on_curve {
            (last.doubled(), 0, p.len() - 1)
        } else {
            (midpoint(first, last), 0, p.len())
        };
        self.emit(out, OutlineCommand::MoveTo(start), offset)?;
        while i < end {
            let point = p[i];
            i += 1;
            if point.on_curve {
                self.emit(out, OutlineCommand::LineTo(point.doubled()), offset)?;
                continue;
            }
            let mut control = point;
            loop {
                if i == end {
                    self.emit(
                        out,
                        OutlineCommand::QuadTo {
                            control: control.doubled(),
                            end: start,
                        },
                        offset,
                    )?;
                    self.emit(out, OutlineCommand::Close, offset)?;
                    return Ok(());
                }
                let next = p[i];
                i += 1;
                if next.on_curve {
                    self.emit(
                        out,
                        OutlineCommand::QuadTo {
                            control: control.doubled(),
                            end: next.doubled(),
                        },
                        offset,
                    )?;
                    break;
                }
                self.emit(
                    out,
                    OutlineCommand::QuadTo {
                        control: control.doubled(),
                        end: midpoint(control, next),
                    },
                    offset,
                )?;
                control = next;
            }
        }
        self.emit(out, OutlineCommand::LineTo(start), offset)?;
        self.emit(out, OutlineCommand::Close, offset)
    }
}
fn midpoint(a: FontPoint, b: FontPoint) -> FontPoint2 {
    FontPoint2 {
        x2: a.x + b.x,
        y2: a.y + b.y,
    }
}
fn coordinate(
    c: &mut Cursor<'_>,
    flag: u8,
    short: u8,
    same: u8,
    prior: i32,
) -> Result<i32, OutlineError> {
    let offset = c.offset();
    let delta = if flag & short != 0 {
        let x = i32::from(c.byte()?);
        if flag & same != 0 { x } else { -x }
    } else if flag & same != 0 {
        0
    } else {
        i32::from(c.signed()?)
    };
    let next = prior
        .checked_add(delta)
        .ok_or_else(|| malformed(offset, "coordinate overflow"))?;
    if i16::try_from(next).is_err() {
        return Err(malformed(
            offset,
            "coordinate outside signed 16-bit font units",
        ));
    }
    Ok(next)
}
