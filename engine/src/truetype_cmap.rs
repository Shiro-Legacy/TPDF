// SPDX-License-Identifier: AGPL-3.0-or-later
//! Explicit, allocation-free format-4 sfnt character-code lookup.
//!
//! This does not interpret PDF encodings or ToUnicode, select a preferred cmap,
//! retry symbol codes, shape text, or substitute a missing glyph. Record order,
//! duplicates and aliases are preserved; callers choose one exact record index.
//! Unselected subtable formats are not interpreted. Format-4 search accelerator
//! fields are ignored: binary search uses the validated ordered segment arrays.
//! Semantics follow the OpenType `cmap` specification (Microsoft Typography).
use crate::truetype_outline::{GlyphId, TrueTypeFont};
use std::{fmt, ops::Range};

#[derive(Clone, Copy, Debug)]
pub struct CmapLimits {
    pub max_records: usize,
    pub max_selection_attempts: usize,
    /// Cumulative validated segments, including repeated selections.
    pub max_segments: usize,
    /// Cumulative covered-code validation, including repeated selections.
    pub max_validation_codes: usize,
    pub max_queries: usize,
    /// One unit per header, directory record, selection/query admission,
    /// validated segment/code, and binary-search comparison.
    pub max_work: usize,
}
impl Default for CmapLimits {
    fn default() -> Self {
        Self {
            max_records: 256,
            max_selection_attempts: 1024,
            max_segments: 8192,
            max_validation_codes: 2_000_000,
            max_queries: 1_000_000,
            max_work: 16_000_000,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CmapUsage {
    pub directory_records: usize,
    pub selection_attempts: usize,
    pub segments: usize,
    pub validated_codes: usize,
    pub queries: usize,
    pub work: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CmapErrorKind {
    MissingTable,
    NoSelection,
    RecordIndex,
    UnsupportedFormat(u16),
    UnsupportedLanguage(u16),
    Invalid(&'static str),
    LimitExceeded(&'static str),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CmapError {
    /// Absolute byte offset in the original font bytes.
    pub offset: usize,
    pub record_index: Option<usize>,
    pub kind: CmapErrorKind,
}
impl fmt::Display for CmapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TrueType cmap record {:?} at {}: {:?}",
            self.record_index, self.offset, self.kind
        )
    }
}
impl std::error::Error for CmapError {}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CmapRecord {
    index: usize,
    platform_id: u16,
    encoding_id: u16,
    format: u16,
    subtable_offset: usize,
}
impl CmapRecord {
    pub fn index(self) -> usize {
        self.index
    }
    pub fn platform_id(self) -> u16 {
        self.platform_id
    }
    pub fn encoding_id(self) -> u16 {
        self.encoding_id
    }
    pub fn format(self) -> u16 {
        self.format
    }
    /// Offset relative to the beginning of cmap, not the font file.
    pub fn subtable_offset(self) -> usize {
        self.subtable_offset
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CmapSelection {
    record: CmapRecord,
    language: u16,
    segment_count: usize,
    cmap_range: Range<usize>,
    subtable_range: Range<usize>,
}
impl CmapSelection {
    pub fn record(&self) -> CmapRecord {
        self.record
    }
    pub fn language(&self) -> u16 {
        self.language
    }
    pub fn segment_count(&self) -> usize {
        self.segment_count
    }
    pub fn cmap_range(&self) -> Range<usize> {
        self.cmap_range.clone()
    }
    pub fn subtable_range(&self) -> Range<usize> {
        self.subtable_range.clone()
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CmapLookup {
    Missing,
    Glyph(GlyphId),
}
#[derive(Clone, Debug)]
struct Selected {
    provenance: CmapSelection,
    end_codes: usize,
    start_codes: usize,
    deltas: usize,
    offsets: usize,
    glyphs: usize,
    end: usize,
}
/// Cmap bytes borrow the already validated sfnt. Font opening and its index
/// allocation are an external prerequisite bounded by O0's FontLimits.
#[derive(Debug)]
pub struct TrueTypeCmapSession<'font, 'bytes> {
    font: &'font TrueTypeFont<'bytes>,
    bytes: &'bytes [u8],
    base: usize,
    records: usize,
    limits: CmapLimits,
    usage: CmapUsage,
    selected: Option<Selected>,
}
fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated cmap header"),
    )
}
fn add(
    value: &mut usize,
    amount: usize,
    cap: usize,
    offset: usize,
    record_index: Option<usize>,
    what: &'static str,
) -> Result<(), CmapError> {
    *value = value
        .checked_add(amount)
        .filter(|n| *n <= cap)
        .ok_or(CmapError {
            offset,
            record_index,
            kind: CmapErrorKind::LimitExceeded(what),
        })?;
    Ok(())
}
impl<'font, 'bytes> TrueTypeCmapSession<'font, 'bytes> {
    pub fn new(font: &'font TrueTypeFont<'bytes>, limits: CmapLimits) -> Result<Self, CmapError> {
        let table = font
            .table_records()
            .iter()
            .find(|r| r.tag() == *b"cmap")
            .ok_or(CmapError {
                offset: 0,
                record_index: None,
                kind: CmapErrorKind::MissingTable,
            })?;
        let bytes = font.table_data(*b"cmap").expect("table record found");
        let mut session = Self {
            font,
            bytes,
            base: table.offset(),
            records: 0,
            limits,
            usage: CmapUsage::default(),
            selected: None,
        };
        session.work(1, 0, None)?;
        if bytes.len() < 4 {
            return Err(session.error(0, None, CmapErrorKind::Invalid("truncated header")));
        }
        if u16_at(bytes, 0) != 0 {
            return Err(session.error(0, None, CmapErrorKind::Invalid("cmap version")));
        }
        session.records = usize::from(u16_at(bytes, 2));
        if session.records > limits.max_records {
            return Err(session.error(2, None, CmapErrorKind::LimitExceeded("encoding records")));
        }
        let directory_end = 4 + session.records * 8; // u16 count cannot overflow usize on supported targets
        if directory_end > bytes.len() {
            return Err(session.error(
                2,
                None,
                CmapErrorKind::Invalid("truncated encoding records"),
            ));
        }
        for index in 0..session.records {
            session.work(1, 4 + index * 8, Some(index))?;
            session.usage.directory_records += 1;
            let at = 4 + index * 8 + 4;
            let offset = usize::try_from(u32_at(bytes, at)).map_err(|_| {
                session.error(
                    at,
                    Some(index),
                    CmapErrorKind::Invalid("subtable offset overflow"),
                )
            })?;
            if offset < directory_end || offset.checked_add(2).is_none_or(|end| end > bytes.len()) {
                return Err(session.error(
                    at,
                    Some(index),
                    CmapErrorKind::Invalid("subtable offset"),
                ));
            }
        }
        Ok(session)
    }
    pub fn records(&self) -> impl ExactSizeIterator<Item = CmapRecord> + '_ {
        (0..self.records).map(|i| self.record(i))
    }
    fn record(&self, index: usize) -> CmapRecord {
        let at = 4 + index * 8;
        let subtable_offset = u32_at(self.bytes, at + 4) as usize;
        CmapRecord {
            index,
            platform_id: u16_at(self.bytes, at),
            encoding_id: u16_at(self.bytes, at + 2),
            format: u16_at(self.bytes, subtable_offset),
            subtable_offset,
        }
    }
    pub fn usage(&self) -> CmapUsage {
        self.usage
    }
    pub fn selection(&self) -> Option<&CmapSelection> {
        self.selected.as_ref().map(|s| &s.provenance)
    }
    fn error(&self, offset: usize, record_index: Option<usize>, kind: CmapErrorKind) -> CmapError {
        CmapError {
            offset: self.base + offset,
            record_index,
            kind,
        }
    }
    fn work(
        &mut self,
        amount: usize,
        offset: usize,
        index: Option<usize>,
    ) -> Result<(), CmapError> {
        add(
            &mut self.usage.work,
            amount,
            self.limits.max_work,
            self.base + offset,
            index,
            "work",
        )
    }
    /// Selects exactly this directory record. Every failed attempt clears the
    /// prior selection; spent validation/query work is never refunded.
    pub fn select(&mut self, index: usize) -> Result<CmapSelection, CmapError> {
        self.selected = None;
        add(
            &mut self.usage.selection_attempts,
            1,
            self.limits.max_selection_attempts,
            self.base,
            Some(index),
            "selection attempts",
        )?;
        self.work(1, 0, Some(index))?;
        if index >= self.records {
            return Err(self.error(0, Some(index), CmapErrorKind::RecordIndex));
        }
        let record = self.record(index);
        let offset = record.subtable_offset;
        if record.format != 4 {
            return Err(self.error(
                offset,
                Some(index),
                CmapErrorKind::UnsupportedFormat(record.format),
            ));
        }
        if offset
            .checked_add(16)
            .is_none_or(|end| end > self.bytes.len())
        {
            return Err(self.error(
                offset,
                Some(index),
                CmapErrorKind::Invalid("truncated format 4 header"),
            ));
        }
        let length = usize::from(u16_at(self.bytes, offset + 2));
        let language = u16_at(self.bytes, offset + 4);
        if language != 0 {
            return Err(self.error(
                offset + 4,
                Some(index),
                CmapErrorKind::UnsupportedLanguage(language),
            ));
        }
        let seg_count_x2 = usize::from(u16_at(self.bytes, offset + 6));
        if seg_count_x2 == 0 || seg_count_x2 % 2 != 0 {
            return Err(self.error(
                offset + 6,
                Some(index),
                CmapErrorKind::Invalid("segCountX2"),
            ));
        }
        let count = seg_count_x2 / 2;
        if count > self.limits.max_segments - self.usage.segments {
            return Err(self.error(
                offset + 6,
                Some(index),
                CmapErrorKind::LimitExceeded("segments"),
            ));
        }
        let end = offset
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| {
                self.error(
                    offset + 2,
                    Some(index),
                    CmapErrorKind::Invalid("subtable length"),
                )
            })?;
        if length % 2 != 0 || length < 16 + 8 * count {
            return Err(self.error(
                offset + 2,
                Some(index),
                CmapErrorKind::Invalid("format 4 arrays/length"),
            ));
        }
        if u16_at(self.bytes, offset + 14 + 2 * count) != 0 {
            return Err(self.error(
                offset + 14 + 2 * count,
                Some(index),
                CmapErrorKind::Invalid("reservedPad"),
            ));
        }
        let selected = Selected {
            provenance: CmapSelection {
                record,
                language,
                segment_count: count,
                cmap_range: self.base..self.base + self.bytes.len(),
                subtable_range: self.base + offset..self.base + end,
            },
            end_codes: offset + 14,
            start_codes: offset + 16 + 2 * count,
            deltas: offset + 16 + 4 * count,
            offsets: offset + 16 + 6 * count,
            glyphs: offset + 16 + 8 * count,
            end,
        };
        let mut previous_end = None;
        for segment in 0..count {
            self.work(1, selected.end_codes + 2 * segment, Some(index))?;
            // Admission above caps the entire selection before segment work.
            self.usage.segments += 1;
            let start = u16_at(self.bytes, selected.start_codes + 2 * segment);
            let end_code = u16_at(self.bytes, selected.end_codes + 2 * segment);
            if start > end_code || previous_end.is_some_and(|last| start <= last) {
                return Err(self.error(
                    selected.start_codes + 2 * segment,
                    Some(index),
                    CmapErrorKind::Invalid("unordered or overlapping segments"),
                ));
            }
            previous_end = Some(end_code);
            if segment + 1 == count && (start != 0xffff || end_code != 0xffff) {
                return Err(self.error(
                    selected.start_codes + 2 * segment,
                    Some(index),
                    CmapErrorKind::Invalid("missing final FFFF sentinel"),
                ));
            }
            let range_at = selected.offsets + 2 * segment;
            let range = usize::from(u16_at(self.bytes, range_at));
            if range != 0 {
                let first = range_at.checked_add(range).ok_or_else(|| {
                    self.error(
                        range_at,
                        Some(index),
                        CmapErrorKind::Invalid("idRangeOffset overflow"),
                    )
                })?;
                let last_end = first
                    .checked_add(2 * usize::from(end_code - start) + 2)
                    .ok_or_else(|| {
                        self.error(
                            range_at,
                            Some(index),
                            CmapErrorKind::Invalid("glyph array overflow"),
                        )
                    })?;
                if range % 2 != 0 || first < selected.glyphs || last_end > selected.end {
                    return Err(self.error(
                        range_at,
                        Some(index),
                        CmapErrorKind::Invalid("idRangeOffset glyph array bounds"),
                    ));
                }
            }
            for code in u32::from(start)..=u32::from(end_code) {
                add(
                    &mut self.usage.validated_codes,
                    1,
                    self.limits.max_validation_codes,
                    self.base + range_at,
                    Some(index),
                    "validation codes",
                )?;
                self.work(1, range_at, Some(index))?;
                let gid = self.raw_lookup(&selected, segment, code as u16);
                if gid >= self.font.glyph_count() {
                    return Err(self.error(
                        range_at,
                        Some(index),
                        CmapErrorKind::Invalid("mapped glyph ID exceeds maxp.numGlyphs"),
                    ));
                }
                if code == 0xffff && gid != 0 {
                    return Err(self.error(
                        range_at,
                        Some(index),
                        CmapErrorKind::Invalid("FFFF sentinel must map to missing glyph"),
                    ));
                }
            }
        }
        let provenance = selected.provenance.clone();
        self.selected = Some(selected);
        Ok(provenance)
    }
    fn raw_lookup(&self, selected: &Selected, segment: usize, code: u16) -> u16 {
        let range_at = selected.offsets + 2 * segment;
        let range = usize::from(u16_at(self.bytes, range_at));
        let delta = u16_at(self.bytes, selected.deltas + 2 * segment);
        if range == 0 {
            return code.wrapping_add(delta);
        }
        let start = u16_at(self.bytes, selected.start_codes + 2 * segment);
        let raw = u16_at(self.bytes, range_at + range + 2 * usize::from(code - start));
        if raw == 0 { 0 } else { raw.wrapping_add(delta) }
    }
    /// Lookup uses a 16-bit code in the explicitly chosen record's encoding.
    /// In particular, Symbol and Macintosh codes are not Unicode scalars here.
    pub fn lookup(&mut self, code: u16) -> Result<CmapLookup, CmapError> {
        let index = self.selected.as_ref().map(|s| s.provenance.record.index);
        add(
            &mut self.usage.queries,
            1,
            self.limits.max_queries,
            self.base,
            index,
            "queries",
        )?;
        self.work(1, 0, index)?;
        let selected = self
            .selected
            .clone()
            .ok_or_else(|| self.error(0, None, CmapErrorKind::NoSelection))?;
        let mut low = 0;
        let mut high = selected.provenance.segment_count;
        while low < high {
            let middle = low + (high - low) / 2;
            self.work(1, selected.end_codes + 2 * middle, index)?;
            if u16_at(self.bytes, selected.end_codes + 2 * middle) < code {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        if low == selected.provenance.segment_count
            || code < u16_at(self.bytes, selected.start_codes + 2 * low)
        {
            return Ok(CmapLookup::Missing);
        }
        let gid = self.raw_lookup(&selected, low, code);
        if gid == 0 {
            return Ok(CmapLookup::Missing);
        }
        // Every possible returned ID was validated at selection, against this
        // same borrowed font's immutable glyph count.
        Ok(CmapLookup::Glyph(
            self.font
                .glyph_id(u32::from(gid))
                .expect("validated cmap glyph ID"),
        ))
    }
}
