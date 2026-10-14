// SPDX-License-Identifier: AGPL-3.0-or-later
//! Allocation-free horizontal TrueType metrics from a validated immutable font.
//! These font-program advances are not PDF Widths. Drawing-origin normalization
//! uses lsb minus raw glyf header xMin, separately from PDF text advancement.
use crate::truetype_outline::{GlyphId, TrueTypeFont};
use std::{fmt, ops::Range};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetricsError {
    pub table: [u8; 4],
    pub offset: usize,
    pub reason: &'static str,
}
impl fmt::Display for MetricsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TrueType metrics {:?} at {}: {}",
            self.table, self.offset, self.reason
        )
    }
}
impl std::error::Error for MetricsError {}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HorizontalMetric {
    advance: u16,
    lsb: i16,
}
impl HorizontalMetric {
    pub fn advance_width(self) -> u16 {
        self.advance
    }
    pub fn left_side_bearing(self) -> i16 {
        self.lsb
    }
}
#[derive(Debug)]
pub struct HorizontalMetrics<'a> {
    bytes: &'a [u8],
    glyphs: u16,
    metrics: u16,
    hhea: Range<usize>,
    hmtx: Range<usize>,
}
fn word(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}
fn table<'a>(
    font: &TrueTypeFont<'a>,
    tag: [u8; 4],
) -> Result<(&'a [u8], Range<usize>), MetricsError> {
    let record = font
        .table_records()
        .iter()
        .find(|r| r.tag() == tag)
        .ok_or(MetricsError {
            table: tag,
            offset: 0,
            reason: "missing table",
        })?;
    let bytes = font.table_data(tag).ok_or(MetricsError {
        table: tag,
        offset: record.offset(),
        reason: "invalid directory",
    })?;
    Ok((bytes, record.range()))
}
fn length(bytes: &[u8], minimum: usize, tag: [u8; 4], base: usize) -> Result<(), MetricsError> {
    if bytes.len() < minimum {
        return Err(MetricsError {
            table: tag,
            offset: base + bytes.len(),
            reason: "truncated table",
        });
    }
    if bytes.len() - minimum > 3 || bytes[minimum..].iter().any(|b| *b != 0) {
        return Err(MetricsError {
            table: tag,
            offset: base + minimum,
            reason: "nonzero or excessive trailing padding",
        });
    }
    Ok(())
}
impl<'a> HorizontalMetrics<'a> {
    /// Constant metadata inspection plus at most two bounded font-directory scans.
    /// No glyph-array traversal or heap allocation. Queries read at most two words.
    pub fn open(font: &TrueTypeFont<'a>) -> Result<Self, MetricsError> {
        let (hhea, hr) = table(font, *b"hhea")?;
        length(hhea, 36, *b"hhea", hr.start)?;
        if hhea[..4] != [0, 1, 0, 0] {
            return Err(MetricsError {
                table: *b"hhea",
                offset: hr.start,
                reason: "unsupported version",
            });
        }
        if hhea[24..34].iter().any(|b| *b != 0) {
            return Err(MetricsError {
                table: *b"hhea",
                offset: hr.start + 24,
                reason: "reserved fields or metricDataFormat",
            });
        }
        let metrics = word(hhea, 34);
        let glyphs = font.glyph_count();
        if metrics == 0 || metrics > glyphs {
            return Err(MetricsError {
                table: *b"hhea",
                offset: hr.start + 34,
                reason: "numberOfHMetrics outside glyph range",
            });
        }
        let (bytes, mr) = table(font, *b"hmtx")?;
        // All counts are u16 and this expression is at most262140.
        let expected = 4 * usize::from(metrics) + 2 * usize::from(glyphs - metrics);
        length(bytes, expected, *b"hmtx", mr.start)?;
        Ok(Self {
            bytes,
            glyphs,
            metrics,
            hhea: hr,
            hmtx: mr,
        })
    }
    pub fn hhea_range(&self) -> Range<usize> {
        self.hhea.clone()
    }
    pub fn hmtx_range(&self) -> Range<usize> {
        self.hmtx.clone()
    }
    pub fn number_of_h_metrics(&self) -> u16 {
        self.metrics
    }
    pub fn get(&self, gid: GlyphId) -> Result<HorizontalMetric, MetricsError> {
        let raw = gid.get();
        if raw >= self.glyphs {
            return Err(MetricsError {
                table: *b"hmtx",
                offset: self.hmtx.start,
                reason: "GID outside font glyph range",
            });
        }
        let index = usize::from(raw);
        let count = usize::from(self.metrics);
        let advance_at = 4 * index.min(count - 1);
        let lsb_at = if index < count {
            4 * index + 2
        } else {
            4 * count + 2 * (index - count)
        };
        Ok(HorizontalMetric {
            advance: word(self.bytes, advance_at),
            lsb: word(self.bytes, lsb_at) as i16,
        })
    }
}
