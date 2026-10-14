// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded rational line scan conversion on a 1/256 output-pixel lattice.
//! Dimensions and emitted spans are in the four-times supersampled grid.
//! All input contours must already have explicit closing edges. Equal crossings
//! are grouped exactly; no off-canvas intersection is discarded before winding.
use crate::path::FillRule;
use std::{fmt, mem::size_of};

pub const MAX_COORDINATE: i32 = 1 << 29;
pub const MAX_SAMPLE_DIMENSION: u32 = 1 << 23;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanPoint {
    pub x: i32,
    pub y: i32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanEdge {
    pub start: ScanPoint,
    pub end: ScanPoint,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanSpan {
    pub y: u32,
    pub x0: u32,
    pub x1: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct ScanLimits {
    pub max_edges: usize,
    pub max_rows: usize,
    pub max_work: u64,
    pub max_allocated_bytes: usize,
    pub max_spans: usize,
    pub max_samples: u64,
}
impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_edges: 1_000_000,
            max_rows: 1_000_000,
            max_work: 200_000_000,
            max_allocated_bytes: 64 * 1024 * 1024,
            max_spans: 10_000_000,
            max_samples: 500_000_000,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScanUsage {
    pub edges: usize,
    pub rows: usize,
    pub work: u64,
    pub allocated_bytes: usize,
    pub spans: usize,
    pub samples: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanError {
    Geometry,
    Limit(&'static str),
    Allocation,
    Arithmetic,
}
impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "scan conversion error: {self:?}")
    }
}
impl std::error::Error for ScanError {}
#[derive(Clone, Copy, Debug)]
struct Crossing {
    num: i64,
    den: i64,
    winding: i64,
}
impl Crossing {
    fn compare(self, other: Self) -> Result<std::cmp::Ordering, ScanError> {
        let a = i128::from(self.num)
            .checked_mul(i128::from(other.den))
            .ok_or(ScanError::Arithmetic)?;
        let b = i128::from(other.num)
            .checked_mul(i128::from(self.den))
            .ok_or(ScanError::Arithmetic)?;
        Ok(a.cmp(&b))
    }
    fn first_sample(self, width: u32) -> Result<u32, ScanError> {
        let offset = self.den.checked_mul(32).ok_or(ScanError::Arithmetic)?;
        let n = self.num.checked_sub(offset).ok_or(ScanError::Arithmetic)?;
        let d = self.den.checked_mul(64).ok_or(ScanError::Arithmetic)?;
        let ceil = n
            .div_euclid(d)
            .checked_add(i64::from(n.rem_euclid(d) != 0))
            .ok_or(ScanError::Arithmetic)?;
        Ok(ceil.clamp(0, i64::from(width)) as u32)
    }
}
fn charge_usize(
    value: &mut usize,
    amount: usize,
    max: usize,
    name: &'static str,
) -> Result<(), ScanError> {
    let next = value.checked_add(amount).ok_or(ScanError::Limit(name))?;
    if next > max {
        return Err(ScanError::Limit(name));
    }
    *value = next;
    Ok(())
}
fn charge_u64(value: &mut u64, amount: u64, max: u64, name: &'static str) -> Result<(), ScanError> {
    let next = value.checked_add(amount).ok_or(ScanError::Limit(name))?;
    if next > max {
        return Err(ScanError::Limit(name));
    }
    *value = next;
    Ok(())
}
/// The scratch arena and usage ledger survive successful and failed calls.
/// Callback invocations are charged for their span samples before invocation.
/// Callbacks should only write into previously reserved, dimension-checked memory.
#[derive(Debug)]
pub struct ScanSession {
    limits: ScanLimits,
    usage: ScanUsage,
    crossings: Vec<Crossing>,
}
impl ScanSession {
    pub fn new(limits: ScanLimits) -> Self {
        Self {
            limits,
            usage: ScanUsage::default(),
            crossings: Vec::new(),
        }
    }
    pub fn usage(&self) -> ScanUsage {
        self.usage
    }
    fn work(&mut self, amount: u64) -> Result<(), ScanError> {
        charge_u64(&mut self.usage.work, amount, self.limits.max_work, "work")
    }
    fn compare(&mut self, a: usize, b: usize) -> Result<std::cmp::Ordering, ScanError> {
        self.work(1)?;
        self.crossings[a].compare(self.crossings[b])
    }
    fn swap(&mut self, a: usize, b: usize) -> Result<(), ScanError> {
        self.work(1)?;
        self.crossings.swap(a, b);
        Ok(())
    }
    fn sift(&mut self, mut root: usize, end: usize) -> Result<(), ScanError> {
        loop {
            self.work(1)?;
            let Some(mut child) = root.checked_mul(2).and_then(|x| x.checked_add(1)) else {
                return Err(ScanError::Arithmetic);
            };
            if child >= end {
                return Ok(());
            }
            if child + 1 < end && self.compare(child, child + 1)?.is_lt() {
                child += 1;
            }
            if !self.compare(root, child)?.is_lt() {
                return Ok(());
            }
            self.swap(root, child)?;
            root = child;
        }
    }
    fn sort(&mut self) -> Result<(), ScanError> {
        let len = self.crossings.len();
        for root in (0..len / 2).rev() {
            self.sift(root, len)?;
        }
        for end in (1..len).rev() {
            self.swap(0, end)?;
            self.sift(0, end)?;
        }
        Ok(())
    }
    pub fn scan(
        &mut self,
        edges: &[ScanEdge],
        rule: FillRule,
        sample_width: u32,
        sample_height: u32,
        mut emit: impl FnMut(ScanSpan),
    ) -> Result<(), ScanError> {
        self.work(1)?;
        if sample_width == 0
            || sample_height == 0
            || sample_width > MAX_SAMPLE_DIMENSION
            || sample_height > MAX_SAMPLE_DIMENSION
        {
            return Err(ScanError::Geometry);
        }
        charge_usize(
            &mut self.usage.edges,
            edges.len(),
            self.limits.max_edges,
            "edges",
        )?;
        let mut min_y = MAX_COORDINATE;
        let mut max_y = -MAX_COORDINATE;
        let mut nonhorizontal = 0usize;
        for edge in edges {
            self.work(1)?;
            for point in [edge.start, edge.end] {
                if !(-MAX_COORDINATE..=MAX_COORDINATE).contains(&point.x)
                    || !(-MAX_COORDINATE..=MAX_COORDINATE).contains(&point.y)
                {
                    return Err(ScanError::Geometry);
                }
            }
            if edge.start.y != edge.end.y {
                nonhorizontal += 1;
                min_y = min_y.min(edge.start.y).min(edge.end.y);
                max_y = max_y.max(edge.start.y).max(edge.end.y);
            }
        }
        if nonhorizontal == 0 {
            return Ok(());
        }
        let first_y = Crossing {
            num: i64::from(min_y),
            den: 1,
            winding: 0,
        }
        .first_sample(sample_height)?;
        let last_y = Crossing {
            num: i64::from(max_y),
            den: 1,
            winding: 0,
        }
        .first_sample(sample_height)?;
        if first_y >= last_y {
            return Ok(());
        }
        self.crossings.clear();
        if self.crossings.capacity() < nonhorizontal {
            let bytes = nonhorizontal
                .checked_mul(size_of::<Crossing>())
                .ok_or(ScanError::Limit("allocated_bytes"))?;
            charge_usize(
                &mut self.usage.allocated_bytes,
                bytes,
                self.limits.max_allocated_bytes,
                "allocated_bytes",
            )?;
            self.crossings
                .try_reserve_exact(nonhorizontal)
                .map_err(|_| ScanError::Allocation)?;
        }
        for y in first_y..last_y {
            charge_usize(&mut self.usage.rows, 1, self.limits.max_rows, "rows")?;
            self.work(1)?;
            self.crossings.clear();
            let sample_y = i64::from(y)
                .checked_mul(64)
                .and_then(|v| v.checked_add(32))
                .ok_or(ScanError::Arithmetic)?;
            for edge in edges {
                self.work(1)?;
                let (a, b, winding) = if edge.start.y < edge.end.y {
                    (edge.start, edge.end, 1)
                } else {
                    (edge.end, edge.start, -1)
                };
                if sample_y < i64::from(a.y) || sample_y >= i64::from(b.y) {
                    continue;
                }
                let dx = i64::from(b.x) - i64::from(a.x);
                let dy = i64::from(b.y) - i64::from(a.y);
                let num = i64::from(a.x)
                    .checked_mul(dy)
                    .and_then(|left| {
                        (sample_y - i64::from(a.y))
                            .checked_mul(dx)
                            .and_then(|right| left.checked_add(right))
                    })
                    .ok_or(ScanError::Arithmetic)?;
                self.crossings.push(Crossing {
                    num,
                    den: dy,
                    winding,
                });
            }
            self.sort()?;
            let mut i = 0;
            let mut winding = 0i64;
            let mut parity = false;
            let mut previous: Option<Crossing> = None;
            while i < self.crossings.len() {
                self.work(1)?;
                let crossing = self.crossings[i];
                let inside = match rule {
                    FillRule::NonZero => winding != 0,
                    FillRule::EvenOdd => parity,
                };
                if inside {
                    let left = previous.ok_or(ScanError::Arithmetic)?;
                    self.work(1)?;
                    let x0 = left.first_sample(sample_width)?;
                    let x1 = crossing.first_sample(sample_width)?;
                    if x0 < x1 {
                        charge_usize(&mut self.usage.spans, 1, self.limits.max_spans, "spans")?;
                        let count = u64::from(x1 - x0);
                        charge_u64(
                            &mut self.usage.samples,
                            count,
                            self.limits.max_samples,
                            "samples",
                        )?;
                        self.work(count)?;
                        emit(ScanSpan { y, x0, x1 });
                    }
                }
                let mut sum = 0i64;
                let mut odd = false;
                loop {
                    self.work(1)?;
                    sum = sum
                        .checked_add(self.crossings[i].winding)
                        .ok_or(ScanError::Arithmetic)?;
                    odd = !odd;
                    i += 1;
                    if i == self.crossings.len() {
                        break;
                    }
                    self.work(1)?;
                    if crossing.compare(self.crossings[i])? != std::cmp::Ordering::Equal {
                        break;
                    }
                }
                winding = winding.checked_add(sum).ok_or(ScanError::Arithmetic)?;
                parity ^= odd;
                previous = Some(crossing);
            }
            // Closed contours have no residual winding to the right of every
            // crossing. Reject detectable violations of the caller contract.
            if winding != 0 || parity {
                return Err(ScanError::Geometry);
            }
        }
        Ok(())
    }
}
