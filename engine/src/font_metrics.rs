// SPDX-License-Identifier: AGPL-3.0-or-later
// Explicit PDF width/default behavior references MuPDF 1.28.1
// source/pdf/pdf-font.c (Artifex 2004-2026) and pdf-metrics.c
// (Artifex 2004-2021). See NOTICE.
//! Finite horizontal simple-font advances in units of 1/1000 text space.
//! These are PDF declarations, not glyph selection, shaping or text positioning.
//! Explicit zero/negative/fractional widths are preserved. Out-of-range codes use
//! the declared MissingWidth, or a provenance-bearing default zero.
use std::{error::Error, fmt};
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontAdvance(f64);
impl FontAdvance {
    pub fn new(value: f64) -> Result<Self, MetricsError> {
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(MetricsError {
                index: None,
                kind: MetricsErrorKind::NonFinite,
            })
        }
    }
    pub fn from_integer(value: i64) -> Result<Self, MetricsError> {
        if !(-(1i64 << 53)..=(1i64 << 53)).contains(&value) {
            return Err(MetricsError {
                index: None,
                kind: MetricsErrorKind::InexactInteger,
            });
        }
        Ok(Self(value as f64))
    }
    pub fn value(self) -> f64 {
        self.0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvanceOrigin {
    WidthsEntry,
    DeclaredMissingWidth,
    DefaultMissingWidth,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdvanceLookup {
    advance: FontAdvance,
    origin: AdvanceOrigin,
}
impl AdvanceLookup {
    pub fn advance(self) -> FontAdvance {
        self.advance
    }
    pub fn origin(self) -> AdvanceOrigin {
        self.origin
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct HorizontalMetrics {
    first: u8,
    last: u8,
    widths: Vec<FontAdvance>,
    missing: AdvanceLookup,
    /// CIDFont /W ranges sorted by first CID, nonoverlapping; None for simple fonts.
    cid: Option<Vec<CidWidthRange>>,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CidWidthRange {
    pub first: u32,
    pub last: u32,
    pub advance: FontAdvance,
}
impl HorizontalMetrics {
    pub fn from_widths(
        first: u8,
        last: u8,
        widths: &[FontAdvance],
        missing: Option<FontAdvance>,
    ) -> Result<Self, MetricsError> {
        if first > last {
            return Err(MetricsError {
                index: None,
                kind: MetricsErrorKind::ReversedRange,
            });
        }
        let expected = usize::from(last) - usize::from(first) + 1;
        if widths.len() != expected {
            return Err(MetricsError {
                index: None,
                kind: MetricsErrorKind::WrongWidthCount {
                    expected,
                    actual: widths.len(),
                },
            });
        }
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(expected)
            .map_err(|_| MetricsError {
                index: None,
                kind: MetricsErrorKind::AllocationFailed,
            })?;
        owned.extend_from_slice(widths);
        let missing = match missing {
            Some(advance) => AdvanceLookup {
                advance,
                origin: AdvanceOrigin::DeclaredMissingWidth,
            },
            None => AdvanceLookup {
                advance: FontAdvance(0.0),
                origin: AdvanceOrigin::DefaultMissingWidth,
            },
        };
        Ok(Self {
            first,
            last,
            widths: owned,
            missing,
            cid: None,
        })
    }
    /// CIDFont horizontal metrics: /W ranges plus /DW (default 1000).
    /// Ranges are sorted here; overlapping declarations are refused rather
    /// than resolved by an order-dependent rule.
    pub fn from_cid_widths(
        mut ranges: Vec<CidWidthRange>,
        default: Option<FontAdvance>,
    ) -> Result<Self, MetricsError> {
        ranges.sort_by_key(|r| r.first);
        for (index, pair) in ranges.windows(2).enumerate() {
            if pair[0].first > pair[0].last || pair[1].first <= pair[0].last {
                return Err(MetricsError {
                    index: Some(index),
                    kind: MetricsErrorKind::OverlappingCidRange,
                });
            }
        }
        if ranges.last().is_some_and(|r| r.first > r.last) {
            return Err(MetricsError {
                index: None,
                kind: MetricsErrorKind::ReversedRange,
            });
        }
        let missing = match default {
            Some(advance) => AdvanceLookup {
                advance,
                origin: AdvanceOrigin::DeclaredMissingWidth,
            },
            None => AdvanceLookup {
                advance: FontAdvance(1000.0),
                origin: AdvanceOrigin::DefaultMissingWidth,
            },
        };
        Ok(Self {
            first: 0,
            last: 0,
            widths: Vec::new(),
            missing,
            cid: Some(ranges),
        })
    }
    pub fn is_cid(&self) -> bool {
        self.cid.is_some()
    }
    pub fn cid_ranges(&self) -> &[CidWidthRange] {
        self.cid.as_deref().unwrap_or(&[])
    }
    pub fn lookup_cid(&self, cid: u32) -> AdvanceLookup {
        let ranges = self.cid_ranges();
        let i = ranges.partition_point(|r| r.last < cid);
        match ranges.get(i) {
            Some(r) if r.first <= cid => AdvanceLookup {
                advance: r.advance,
                origin: AdvanceOrigin::WidthsEntry,
            },
            _ => self.missing,
        }
    }
    pub fn first(&self) -> u8 {
        self.first
    }
    pub fn last(&self) -> u8 {
        self.last
    }
    pub fn widths(&self) -> &[FontAdvance] {
        &self.widths
    }
    pub fn missing(&self) -> AdvanceLookup {
        self.missing
    }
    pub fn lookup(&self, code: u8) -> AdvanceLookup {
        if code >= self.first && code <= self.last {
            AdvanceLookup {
                advance: self.widths[usize::from(code) - usize::from(self.first)],
                origin: AdvanceOrigin::WidthsEntry,
            }
        } else {
            self.missing
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricsError {
    pub index: Option<usize>,
    pub kind: MetricsErrorKind,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricsErrorKind {
    NonFinite,
    InexactInteger,
    ReversedRange,
    WrongWidthCount { expected: usize, actual: usize },
    AllocationFailed,
    OverlappingCidRange,
}
impl fmt::Display for MetricsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "horizontal font metrics at width index {:?}: {:?}",
            self.index, self.kind
        )
    }
}
impl Error for MetricsError {}
