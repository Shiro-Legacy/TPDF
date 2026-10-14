// SPDX-License-Identifier: AGPL-3.0-or-later
//! Checked placement of PDF user coordinates into top-left page coordinates.
//! Positive sub-unit boxes keep their dimensions; no native minimum-size repair
//! is applied. This is geometry only: no clipping, raster allocation or rendering.
use crate::geometry::{Affine, DeviceSpace, GeometryError, PageSpace, Rect, UserSpace};
use crate::pages::PageInfo;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageGeometry {
    crop: Rect<UserSpace>,
    bounds: Rect<PageSpace>,
    user_to_page: Affine<UserSpace, PageSpace>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageGeometryError {
    Invalid(&'static str),
    Arithmetic(GeometryError),
}
impl fmt::Display for PageGeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid page geometry: {reason}"),
            Self::Arithmetic(error) => write!(f, "page geometry: {error}"),
        }
    }
}
impl std::error::Error for PageGeometryError {}
impl From<GeometryError> for PageGeometryError {
    fn from(value: GeometryError) -> Self {
        Self::Arithmetic(value)
    }
}
fn checked_box(values: [f64; 4]) -> Result<Rect<UserSpace>, PageGeometryError> {
    let [x0, y0, x1, y1] = values;
    let rect = Rect::from_corners(x0, y0, x1, y1)?;
    if x0 >= x1 || y0 >= y1 {
        return Err(PageGeometryError::Invalid(
            "box must be normalized with positive dimensions",
        ));
    }
    Ok(rect)
}
impl PageGeometry {
    /// Revalidate public inventory fields before producing immutable typed data.
    /// `crop_box` must already be intersected with `media_box`, as collect_pages
    /// provides. Object identity/Contents fields do not affect this calculation.
    pub fn from_page_info(page: &PageInfo) -> Result<Self, PageGeometryError> {
        let media = checked_box(page.media_box)?;
        let crop = checked_box(page.crop_box)?;
        let [mx0, my0, mx1, my1] = media.bounds();
        let [x0, y0, x1, y1] = crop.bounds();
        if x0 < mx0 || y0 < my0 || x1 > mx1 || y1 > my1 {
            return Err(PageGeometryError::Invalid("crop must be inside media box"));
        }
        let unit = page.user_unit;
        if !unit.is_finite() || unit <= 0. || unit > 75_000. {
            return Err(PageGeometryError::Invalid(
                "UserUnit outside supported positive range",
            ));
        }
        // Explicit quarter turns avoid sine/cosine rounding and preserve axes.
        // Coefficients apply PDF rotation, a Y flip, and UserUnit scaling.
        let coefficients = match page.rotation {
            0 => [unit, 0., 0., -unit, -x0 * unit, y1 * unit],
            90 => [0., unit, unit, 0., -y0 * unit, -x0 * unit],
            180 => [-unit, 0., 0., unit, x1 * unit, -y0 * unit],
            270 => [0., -unit, -unit, 0., y1 * unit, x1 * unit],
            _ => {
                return Err(PageGeometryError::Invalid(
                    "rotation must be 0, 90, 180 or 270",
                ));
            }
        };
        let user_to_page = Affine::from_pdf(coefficients)?;
        let transformed = user_to_page.transform_bounds(crop)?;
        if transformed.is_empty() {
            return Err(PageGeometryError::Invalid(
                "transformed dimensions lost precision",
            ));
        }
        let width = crop.width() * unit;
        let height = crop.height() * unit;
        let (width, height) = if page.rotation == 90 || page.rotation == 270 {
            (height, width)
        } else {
            (width, height)
        };
        let bounds = Rect::from_corners(0., 0., width, height)?;
        if bounds.is_empty() {
            return Err(PageGeometryError::Invalid("scaled dimensions underflow"));
        }
        // Translation of a distant origin can lose most of a small page's
        // extent while remaining finite and nonzero. Compare against the
        // expected size using that size, never the much larger source origin.
        let [tx0, ty0, tx1, ty1] = transformed.bounds();
        let x_tolerance = width * (64. * f64::EPSILON);
        let y_tolerance = height * (64. * f64::EPSILON);
        if tx0.abs() > x_tolerance
            || ty0.abs() > y_tolerance
            || (tx1 - width).abs() > x_tolerance
            || (ty1 - height).abs() > y_tolerance
        {
            return Err(PageGeometryError::Invalid(
                "page transform loses extent precision",
            ));
        }
        Ok(Self {
            crop,
            bounds,
            user_to_page,
        })
    }
    pub fn crop(self) -> Rect<UserSpace> {
        self.crop
    }
    pub fn bounds(self) -> Rect<PageSpace> {
        self.bounds
    }
    pub fn user_to_page(self) -> Affine<UserSpace, PageSpace> {
        self.user_to_page
    }
    /// Uniform positive dots-per-inch scale. Pixel rounding/allocation is a
    /// separate decision; this returns continuous geometry only.
    pub fn page_to_device(
        self,
        dpi: f64,
    ) -> Result<Affine<PageSpace, DeviceSpace>, PageGeometryError> {
        if !dpi.is_finite() || dpi <= 0. {
            return Err(PageGeometryError::Invalid(
                "DPI must be finite and positive",
            ));
        }
        let scale = dpi / 72.;
        if scale == 0. {
            return Err(PageGeometryError::Invalid("DPI scale underflow"));
        }
        let transform = Affine::from_pdf([scale, 0., 0., scale, 0., 0.])?;
        let bounds = transform.transform_bounds(self.bounds)?;
        if bounds.is_empty() {
            return Err(PageGeometryError::Invalid("device dimensions underflow"));
        }
        Ok(transform)
    }
}
