// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exact, bounded nearest-neighbor image texel selection. See R23-SAMPLING-DESIGN.md.
//! This module neither decodes images nor paints pages. All predicates operate on
//! the represented final affine coefficients, without path quantization.
use crate::geometry::{Affine, DeviceSpace, UserSpace};
use std::{cmp::Ordering, fmt};

const COORDINATE_CAP: i128 = 1i128 << 101;
const MAX_DIMENSION: u32 = 1 << 16;
const MAX_SAMPLE_INDEX: u32 = 1 << 23;
const PREPARATION_WORK: u64 = 1024;
const SAMPLE_WORK: u64 = 16384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageSamplingError {
    Dimensions,
    NumericDomain,
    Singular,
    ArithmeticInvariant,
    Limit(&'static str),
}
impl fmt::Display for ImageSamplingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "image sampling: {self:?}")
    }
}
impl std::error::Error for ImageSamplingError {}

#[derive(Clone, Copy, Debug)]
pub struct ImageSamplingLimits {
    pub max_preparations: u64,
    pub max_samples: u64,
    /// Conservative fixed arithmetic reservations, not measured CPU instructions.
    pub max_work: u64,
}
impl Default for ImageSamplingLimits {
    fn default() -> Self {
        Self {
            max_preparations: 100_000,
            max_samples: 64_000_000,
            max_work: 1_100_000_000_000,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImageSamplingUsage {
    pub preparations: u64,
    pub samples: u64,
    pub work: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageTexel {
    row: u32,
    column: u32,
}
impl ImageTexel {
    pub fn row(self) -> u32 {
        self.row
    }
    pub fn column(self) -> u32 {
        self.column
    }
}
/// Immutable preparation owns its exact numeric state; no document/session borrow.
#[derive(Clone, Debug)]
pub struct ImagePlacement {
    coefficients: [i128; 6],
    determinant: Wide,
    reflected: bool,
    // Device-space edge ownership for U=0, U=D, V=0, V=D.
    include_edges: [bool; 4],
    width: u32,
    height: u32,
}
impl ImagePlacement {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Conservative half-open candidate window in sample indices. Bounds are
    /// derived from exact lattice corners, never rounded floating-point sums.
    pub fn sample_bounds(&self, width: u32, height: u32) -> [u32; 4] {
        let [a, b, c, d, e, f] = self.coefficients;
        let xs = [e, e + a, e + c, e + a + c];
        let ys = [f, f + b, f + d, f + b + d];
        let bound = |v: i128, cap: u32, upper: bool| {
            (v.div_euclid(1i128 << 78) + i128::from(upper)).clamp(0, i128::from(cap)) as u32
        };
        [
            bound(*xs.iter().min().unwrap(), width, false),
            bound(*ys.iter().min().unwrap(), height, false),
            bound(*xs.iter().max().unwrap(), width, true),
            bound(*ys.iter().max().unwrap(), height, true),
        ]
    }
}
pub struct ImageSamplingSession {
    limits: ImageSamplingLimits,
    usage: ImageSamplingUsage,
}
impl ImageSamplingSession {
    pub fn new(limits: ImageSamplingLimits) -> Self {
        Self {
            limits,
            usage: ImageSamplingUsage::default(),
        }
    }
    pub fn usage(&self) -> ImageSamplingUsage {
        self.usage
    }
    pub fn prepare(
        &mut self,
        transform: Affine<UserSpace, DeviceSpace>,
        width: u32,
        height: u32,
    ) -> Result<ImagePlacement, ImageSamplingError> {
        charge(
            &mut self.usage.preparations,
            self.limits.max_preparations,
            1,
            "preparations",
        )?;
        charge(
            &mut self.usage.work,
            self.limits.max_work,
            PREPARATION_WORK,
            "work",
        )?;
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(ImageSamplingError::Dimensions);
        }
        let mut coefficients = [0; 6];
        for (out, value) in coefficients.iter_mut().zip(transform.coefficients()) {
            *out = lattice(value)?;
        }
        let [a, b, c, d, e, f] = coefficients;
        // All terms are bounded by 2^101; these sums fit i128 before admission.
        for (x, y) in [
            (e, f),
            (e + a, f + b),
            (e + c, f + d),
            (e + a + c, f + b + d),
        ] {
            if x.abs() > COORDINATE_CAP || y.abs() > COORDINATE_CAP {
                return Err(ImageSamplingError::NumericDomain);
            }
        }
        let det = cross(a, b, c, d)?;
        if det.magnitude.is_zero() {
            return Err(ImageSamplingError::Singular);
        }
        let direction = if det.negative { -1 } else { 1 };
        let top_left = |dx: i128, dy: i128| {
            let (dx, dy) = (direction * dx, direction * dy);
            dy < 0 || (dy == 0 && dx > 0)
        };
        Ok(ImagePlacement {
            coefficients,
            determinant: det.magnitude,
            reflected: det.negative,
            include_edges: [
                top_left(-c, -d),
                top_left(c, d),
                top_left(a, b),
                top_left(-a, -b),
            ],
            width,
            height,
        })
    }
    /// Visit the exact 4x sample center ((2*x+1)/8, (2*y+1)/8).
    /// Outside samples and failed numeric admission retain all charged work.
    pub fn sample(
        &mut self,
        placement: &ImagePlacement,
        sample_x: u32,
        sample_y: u32,
    ) -> Result<Option<ImageTexel>, ImageSamplingError> {
        charge(
            &mut self.usage.samples,
            self.limits.max_samples,
            1,
            "samples",
        )?;
        charge(
            &mut self.usage.work,
            self.limits.max_work,
            SAMPLE_WORK,
            "work",
        )?;
        if sample_x >= MAX_SAMPLE_INDEX || sample_y >= MAX_SAMPLE_INDEX {
            return Err(ImageSamplingError::NumericDomain);
        }
        let [a, b, c, d, e, f] = placement.coefficients;
        // Sample coordinates <=2^101, translated differences <=2^102.
        let dx = ((2 * i128::from(sample_x) + 1) << 77) - e;
        let dy = ((2 * i128::from(sample_y) + 1) << 77) - f;
        let mut u = cross(dx, dy, c, d)?;
        let mut v = cross(a, b, dx, dy)?;
        if placement.reflected {
            u = u.negated();
            v = v.negated();
        }
        let determinant = placement.determinant;
        for (value, lo, hi) in [
            (u, placement.include_edges[0], placement.include_edges[1]),
            (v, placement.include_edges[2], placement.include_edges[3]),
        ] {
            if value.negative
                || value.magnitude.cmp(determinant) == Ordering::Greater
                || (value.magnitude.is_zero() && !lo)
                || (value.magnitude == determinant && !hi)
            {
                return Ok(None);
            }
        }
        let row_numerator = determinant.sub(v.magnitude)?;
        Ok(Some(ImageTexel {
            column: texel(u.magnitude, determinant, placement.width)?,
            row: texel(row_numerator, determinant, placement.height)?,
        }))
    }
}
fn charge(
    used: &mut u64,
    limit: u64,
    amount: u64,
    name: &'static str,
) -> Result<(), ImageSamplingError> {
    let next = used
        .checked_add(amount)
        .filter(|v| *v <= limit)
        .ok_or(ImageSamplingError::Limit(name))?;
    *used = next;
    Ok(())
}
// Exact bit conversion, never a rounded floating-point multiplication by 2^80.
fn lattice(value: f64) -> Result<i128, ImageSamplingError> {
    if !value.is_finite() || value.abs() > 2_097_152.0 {
        return Err(ImageSamplingError::NumericDomain);
    }
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    if exponent == 0 {
        return if fraction == 0 {
            Ok(0)
        } else {
            Err(ImageSamplingError::NumericDomain)
        };
    }
    let mantissa = fraction | (1u64 << 52);
    let shift = exponent - 1023 - 52 + 80;
    let scaled = if shift >= 0 {
        // The coordinate guard implies shift<=49, so no significant bit is lost.
        i128::from(mantissa)
            .checked_shl(shift as u32)
            .ok_or(ImageSamplingError::NumericDomain)?
    } else {
        let right = (-shift) as u32;
        if right >= 64 || mantissa.trailing_zeros() < right {
            return Err(ImageSamplingError::NumericDomain);
        }
        i128::from(mantissa >> right)
    };
    if scaled > COORDINATE_CAP {
        return Err(ImageSamplingError::NumericDomain);
    }
    Ok(if bits >> 63 == 0 { scaled } else { -scaled })
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Wide([u64; 4]);
impl Wide {
    fn is_zero(self) -> bool {
        self == Self::default()
    }
    fn cmp(self, other: Self) -> Ordering {
        for i in (0..4).rev() {
            let order = self.0[i].cmp(&other.0[i]);
            if order != Ordering::Equal {
                return order;
            }
        }
        Ordering::Equal
    }
    fn product(a: u128, b: u128) -> Self {
        let a = [a as u64, (a >> 64) as u64];
        let b = [b as u64, (b >> 64) as u64];
        let mut out = [0; 4];
        for (i, &left) in a.iter().enumerate() {
            let mut carry = 0u128;
            for (j, &right) in b.iter().enumerate() {
                // (2^64-1)^2 + 2*(2^64-1) <= u128::MAX.
                let value = u128::from(left) * u128::from(right) + u128::from(out[i + j]) + carry;
                out[i + j] = value as u64;
                carry = value >> 64;
            }
            out[i + 2] = carry as u64;
        }
        Self(out)
    }
    fn add(self, other: Self) -> Result<Self, ImageSamplingError> {
        let mut out = [0; 4];
        let mut carry = 0u128;
        for (i, result) in out.iter_mut().enumerate() {
            let value = u128::from(self.0[i]) + u128::from(other.0[i]) + carry;
            *result = value as u64;
            carry = value >> 64;
        }
        if carry != 0 {
            return Err(ImageSamplingError::ArithmeticInvariant);
        }
        Ok(Self(out))
    }
    fn sub(self, other: Self) -> Result<Self, ImageSamplingError> {
        let mut out = [0; 4];
        let mut borrow = false;
        for (i, result) in out.iter_mut().enumerate() {
            let (value, b1) = self.0[i].overflowing_sub(other.0[i]);
            let (value, b2) = value.overflowing_sub(u64::from(borrow));
            *result = value;
            borrow = b1 || b2;
        }
        if borrow {
            return Err(ImageSamplingError::ArithmeticInvariant);
        }
        Ok(Self(out))
    }
    fn scale(self, factor: u32) -> Result<Self, ImageSamplingError> {
        let mut out = [0; 4];
        let mut carry = 0u128;
        for (i, result) in out.iter_mut().enumerate() {
            let value = u128::from(self.0[i]) * u128::from(factor) + carry;
            *result = value as u64;
            carry = value >> 64;
        }
        if carry != 0 {
            return Err(ImageSamplingError::ArithmeticInvariant);
        }
        Ok(Self(out))
    }
}
#[derive(Clone, Copy)]
struct Signed {
    negative: bool,
    magnitude: Wide,
}
impl Signed {
    fn new(negative: bool, magnitude: Wide) -> Self {
        Self {
            negative: negative && !magnitude.is_zero(),
            magnitude,
        }
    }
    fn product(a: i128, b: i128) -> Self {
        Self::new(
            (a < 0) != (b < 0),
            Wide::product(a.unsigned_abs(), b.unsigned_abs()),
        )
    }
    fn negated(self) -> Self {
        Self::new(!self.negative, self.magnitude)
    }
    fn sub(self, other: Self) -> Result<Self, ImageSamplingError> {
        if self.negative != other.negative {
            return Ok(Self::new(
                self.negative,
                self.magnitude.add(other.magnitude)?,
            ));
        }
        if self.magnitude.cmp(other.magnitude) == Ordering::Less {
            Ok(Self::new(
                !self.negative,
                other.magnitude.sub(self.magnitude)?,
            ))
        } else {
            Ok(Self::new(
                self.negative,
                self.magnitude.sub(other.magnitude)?,
            ))
        }
    }
}
fn cross(a: i128, b: i128, c: i128, d: i128) -> Result<Signed, ImageSamplingError> {
    Signed::product(a, d).sub(Signed::product(b, c))
}
fn texel(numerator: Wide, denominator: Wide, dimension: u32) -> Result<u32, ImageSamplingError> {
    if numerator == denominator {
        return Ok(dimension - 1);
    }
    let target = numerator.scale(dimension)?;
    let (mut lo, mut hi) = (0, dimension);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if denominator.scale(mid)?.cmp(target) == Ordering::Greater {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Ok(lo)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_limb_carry_borrow_and_overflow() {
        let one = Wide([1, 0, 0, 0]);
        let max = Wide([u64::MAX; 4]);
        assert_eq!(max.add(one), Err(ImageSamplingError::ArithmeticInvariant));
        assert_eq!(
            Wide::default().sub(one),
            Err(ImageSamplingError::ArithmeticInvariant)
        );
        let lower = Wide([u64::MAX, u64::MAX, u64::MAX, 0]);
        let upper = Wide([0, 0, 0, 1]);
        assert_eq!(lower.add(one).unwrap(), upper);
        assert_eq!(upper.sub(one).unwrap(), lower);
        assert_eq!(max.scale(2), Err(ImageSamplingError::ArithmeticInvariant));
        assert_eq!(max.scale(0).unwrap(), Wide::default());
    }
    #[test]
    fn full_width_products_and_order() {
        assert_eq!(
            Wide::product(u128::MAX, u128::MAX),
            Wide([1, 0, u64::MAX - 1, u64::MAX])
        );
        assert_eq!(
            Wide::product(1u128 << 100, 1u128 << 100),
            Wide([0, 0, 0, 1 << 8])
        );
        assert_eq!(
            Wide([u64::MAX, 0, 0, 0]).cmp(Wide([0, 1, 0, 0])),
            Ordering::Less
        );
        for a in [
            0,
            1,
            7,
            u64::MAX as u128,
            1u128 << 64,
            1u128 << 100,
            u128::MAX,
        ] {
            assert_eq!(
                Wide::product(a, 1),
                Wide([a as u64, (a >> 64) as u64, 0, 0])
            );
            assert_eq!(Wide::product(a, 0), Wide::default());
        }
    }
    #[test]
    fn signs_and_exact_cancellation() {
        for a in [-17, -1, 0, 1, 17] {
            for b in [-33, -1, 0, 1, 33] {
                let value = Signed::product(a, b);
                let zero = value.sub(value).unwrap();
                assert!(zero.magnitude.is_zero());
                assert!(!zero.negative);
                assert_eq!(value.negative, a * b < 0);
                assert_eq!(value.magnitude, Wide::product((a * b).unsigned_abs(), 1));
            }
        }
        let near = cross(
            1i128 << 80,
            (1i128 << 80) + (1i128 << 28),
            (1i128 << 80) - (1i128 << 28),
            1i128 << 80,
        )
        .unwrap();
        assert!(!near.negative);
        assert_eq!(near.magnitude, Wide([1 << 56, 0, 0, 0]));
    }
    #[test]
    fn bit_conversion_domain_boundaries() {
        assert_eq!(lattice(-0.), Ok(0));
        assert_eq!(lattice(2f64.powi(-80)), Ok(1));
        assert_eq!(lattice(-2f64.powi(-80)), Ok(-1));
        for v in [
            2f64.powi(-81),
            f64::from_bits(1),
            f64::INFINITY,
            f64::NAN,
            2097153.,
        ] {
            assert_eq!(lattice(v), Err(ImageSamplingError::NumericDomain));
        }
        assert_eq!(lattice(2097152.), Ok(COORDINATE_CAP));
        assert_eq!(lattice(-2097152.), Ok(-COORDINATE_CAP));
    }
}
