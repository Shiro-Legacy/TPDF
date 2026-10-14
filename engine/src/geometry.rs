// SPDX-License-Identifier: AGPL-3.0-or-later
//! Finite geometry with compile-time coordinate spaces. Transform composition
//! follows the order a point travels: `first.then(second)` applies first, then
//! second. These primitives do not perform graphics execution or rasterization.
use std::fmt;
use std::marker::PhantomData;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeometryError {
    Nonfinite,
    Singular,
    IntegerBounds,
}
impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Nonfinite => "nonfinite geometry or arithmetic overflow",
            Self::Singular => "singular or numerically unrepresentable inverse transform",
            Self::IntegerBounds => "raster bounds exceed signed 32-bit coordinates",
        })
    }
}
impl std::error::Error for GeometryError {}

// Tags have no runtime data and cannot be confused through implicit conversion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UserSpace {}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageSpace {}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeviceSpace {}

#[derive(Debug, PartialEq)]
pub struct Point<Space> {
    coordinates: [f64; 2],
    space: PhantomData<fn() -> Space>,
}
impl<S> Copy for Point<S> {}
impl<S> Clone for Point<S> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<S> Point<S> {
    pub fn new(x: f64, y: f64) -> Result<Self, GeometryError> {
        finite(&[x, y])?;
        Ok(Self {
            coordinates: [x, y],
            space: PhantomData,
        })
    }
    pub fn coordinates(self) -> [f64; 2] {
        self.coordinates
    }
    pub fn x(self) -> f64 {
        self.coordinates[0]
    }
    pub fn y(self) -> f64 {
        self.coordinates[1]
    }
}
#[derive(Debug, PartialEq)]
pub struct Rect<Space> {
    bounds: [f64; 4],
    space: PhantomData<fn() -> Space>,
}
impl<S> Copy for Rect<S> {}
impl<S> Clone for Rect<S> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<S> Rect<S> {
    /// Both diagonal corner orderings are accepted. Zero-area rectangles are
    /// representable; reversed corners are normalized, never treated as empty.
    pub fn from_corners(x0: f64, y0: f64, x1: f64, y1: f64) -> Result<Self, GeometryError> {
        finite(&[x0, y0, x1, y1])?;
        // Reject a finite pair whose extent cannot be represented as finite f64.
        finite(&[x1 - x0, y1 - y0])?;
        Ok(Self {
            bounds: [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)],
            space: PhantomData,
        })
    }
    pub fn bounds(self) -> [f64; 4] {
        self.bounds
    }
    pub fn width(self) -> f64 {
        self.bounds[2] - self.bounds[0]
    }
    pub fn height(self) -> f64 {
        self.bounds[3] - self.bounds[1]
    }
    pub fn is_empty(self) -> bool {
        self.width() == 0.0 || self.height() == 0.0
    }
    pub fn intersect(self, other: Self) -> Option<Self> {
        let [x0, y0, x1, y1] = self.bounds;
        let [a0, b0, a1, b1] = other.bounds;
        let bounds = [x0.max(a0), y0.max(b0), x1.min(a1), y1.min(b1)];
        if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
            None
        } else {
            Some(Self {
                bounds,
                space: PhantomData,
            })
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RasterBounds {
    bounds: [i32; 4],
}
impl RasterBounds {
    pub fn bounds(self) -> [i32; 4] {
        self.bounds
    }
    pub fn width(self) -> u64 {
        (i64::from(self.bounds[2]) - i64::from(self.bounds[0])) as u64
    }
    pub fn height(self) -> u64 {
        (i64::from(self.bounds[3]) - i64::from(self.bounds[1])) as u64
    }
}
impl Rect<DeviceSpace> {
    /// Outward pixel bounds. Empty continuous rectangles stay empty: rounding
    /// must not turn a zero-width line at a fractional coordinate into pixels.
    pub fn outward_raster_bounds(self) -> Result<RasterBounds, GeometryError> {
        let [x0, y0, x1, y1] = self.bounds;
        let x0i = x0.floor();
        let y0i = y0.floor();
        let rounded = [
            x0i,
            y0i,
            if x0 == x1 { x0i } else { x1.ceil() },
            if y0 == y1 { y0i } else { y1.ceil() },
        ];
        if rounded
            .iter()
            .any(|&v| v < f64::from(i32::MIN) || v > f64::from(i32::MAX))
        {
            return Err(GeometryError::IntegerBounds);
        }
        Ok(RasterBounds {
            bounds: rounded.map(|v| v as i32),
        })
    }
}

/// An affine map with PDF coefficients `[a,b,c,d,e,f]`:
/// `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
///
/// Coordinate spaces cannot be mixed accidentally:
/// ```compile_fail
/// use tandem_pdf_engine::geometry::{Affine,Point,UserSpace,PageSpace,DeviceSpace};
/// let transform=Affine::<UserSpace,DeviceSpace>::from_pdf([1.,0.,0.,1.,0.,0.]).unwrap();
/// let point=Point::<PageSpace>::new(1.,2.).unwrap();
/// transform.transform_point(point).unwrap();
/// ```
/// ```compile_fail
/// use tandem_pdf_engine::geometry::{Affine,UserSpace,PageSpace,DeviceSpace};
/// let first=Affine::<UserSpace,PageSpace>::from_pdf([1.,0.,0.,1.,0.,0.]).unwrap();
/// let wrong=Affine::<DeviceSpace,UserSpace>::from_pdf([1.,0.,0.,1.,0.,0.]).unwrap();
/// first.then(wrong).unwrap();
/// ```
#[derive(Debug, PartialEq)]
pub struct Affine<From, To> {
    coefficients: [f64; 6],
    spaces: PhantomData<fn(From) -> To>,
}
impl<F, T> Copy for Affine<F, T> {}
impl<F, T> Clone for Affine<F, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<F, T> Affine<F, T> {
    pub fn from_pdf(coefficients: [f64; 6]) -> Result<Self, GeometryError> {
        finite(&coefficients)?;
        Ok(Self {
            coefficients,
            spaces: PhantomData,
        })
    }
    pub fn coefficients(self) -> [f64; 6] {
        self.coefficients
    }
    pub fn transform_point(self, point: Point<F>) -> Result<Point<T>, GeometryError> {
        let [a, b, c, d, e, f] = self.coefficients;
        Point::new(
            a * point.x() + c * point.y() + e,
            b * point.x() + d * point.y() + f,
        )
    }
    /// Axis-aligned bounds of the four transformed corners. This transforms
    /// bounds, not a filled region: a zero-width line rotated or sheared can
    /// have a positive-area bounding box. Clipping interpreters must preserve
    /// empty-region semantics separately before using these bounds.
    pub fn transform_bounds(self, rect: Rect<F>) -> Result<Rect<T>, GeometryError> {
        let [x0, y0, x1, y1] = rect.bounds;
        let mut lower = [f64::INFINITY; 2];
        let mut upper = [f64::NEG_INFINITY; 2];
        for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            let p = self.transform_point(Point::new(x, y)?)?.coordinates();
            for axis in 0..2 {
                lower[axis] = lower[axis].min(p[axis]);
                upper[axis] = upper[axis].max(p[axis]);
            }
        }
        Rect::from_corners(lower[0], lower[1], upper[0], upper[1])
    }
    pub fn then<N>(self, next: Affine<T, N>) -> Result<Affine<F, N>, GeometryError> {
        let [a, b, c, d, e, f] = self.coefficients;
        let [g, h, i, j, k, l] = next.coefficients;
        Affine::from_pdf([
            a * g + b * i,
            a * h + b * j,
            c * g + d * i,
            c * h + d * j,
            e * g + f * i + k,
            e * h + f * j + l,
        ])
    }
    /// Inversion uses scaled linear coefficients to avoid overflowing a
    /// determinant of large finite inputs. Determinants within a conservative
    /// f64 rounding-error bound are refused, including some invertible but
    /// ill-conditioned transforms. No exact-arithmetic guarantee is made.
    pub fn inverse(self) -> Result<Affine<T, F>, GeometryError> {
        let [a, b, c, d, e, f] = self.coefficients;
        let scale = a.abs().max(b.abs()).max(c.abs()).max(d.abs());
        if scale == 0.0 {
            return Err(GeometryError::Singular);
        }
        let (a, b, c, d) = (a / scale, b / scale, c / scale, d / scale);
        let diagonal = a * d;
        let off_diagonal = b * c;
        let determinant = diagonal - off_diagonal;
        // Normalization itself rounds: an exactly rank-one matrix can acquire
        // a tiny nonzero determinant. Refuse values within a conservative f64
        // product/subtraction error bound rather than returning a false inverse.
        let uncertainty = 8.0 * f64::EPSILON * (diagonal.abs() + off_diagonal.abs());
        if determinant.abs() <= uncertainty {
            return Err(GeometryError::Singular);
        }
        // Divide in this order to avoid multiplying scale*determinant first.
        let (a, b, c, d) = (
            (d / determinant) / scale,
            (-b / determinant) / scale,
            (-c / determinant) / scale,
            (a / determinant) / scale,
        );
        Affine::from_pdf([a, b, c, d, -a * e - c * f, -b * e - d * f])
            .map_err(|_| GeometryError::Singular)
    }
}
impl<S> Affine<S, S> {
    pub fn identity() -> Self {
        Self {
            coefficients: [1., 0., 0., 1., 0., 0.],
            spaces: PhantomData,
        }
    }
    pub fn translation(x: f64, y: f64) -> Result<Self, GeometryError> {
        Self::from_pdf([1., 0., 0., 1., x, y])
    }
    pub fn scale(x: f64, y: f64) -> Result<Self, GeometryError> {
        Self::from_pdf([x, 0., 0., y, 0., 0.])
    }
}
fn finite(values: &[f64]) -> Result<(), GeometryError> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(GeometryError::Nonfinite)
    }
}
