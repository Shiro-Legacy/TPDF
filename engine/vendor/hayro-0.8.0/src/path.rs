use crate::{Renderer, convert_fill_rule};
use hayro_interpret::{DrawMode, DrawProps, FillRule, StrokeProps};
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape};

// Tandem patch: what one path costs vello at once. It flattens the path into
// lines, sorts the tiles each line crosses and, for a stroke, first expands
// the outline kurbo strokes (a join or cap at every segment, every dash a
// subpath): a 50-byte dash pattern on a page-wide line took 1.4 GB natively,
// and a 24 KB page stroking 2,000,000 tiny segments 2.3 GB of WASM memory
// (8,000,000 aborted the engine). Estimated per segment, per further line a
// curve flattens into (about the square root of its device length) and per
// device pixel of length, as measured on the WASM build (vello_cpu 0.3):
// about 140 bytes a segment of a fill and 1,240 of a stroke (800,000 and
// 400,000 tiny segments), natively 1.5 and 3 to 4 bytes a pixel (20,000
// page-sized curves at 72 and 288 dpi), rounded up. vello culls what lies
// outside the pixmap while it flattens, so lines and pixels count only
// within it (a line or curve 10^24 units long costs what its visible part
// does); kurbo dashes the whole path first, so dashes count along all of
// it. A stroke's outline runs along both sides of each segment and around
// its join, about three widths a segment more (400,000 tiny segments
// stroked 1,000 pixels wide would cross tiles for gigabytes).
/// The estimated bytes one fill, stroke or clip may cost; past it the
/// render is over budget.
pub const MAX_PATH_BYTES: f64 = (1u64 << 29) as f64;
const FILL_SEGMENT_BYTES: f64 = 160.0;
const FILL_LINE_BYTES: f64 = 24.0;
const FILL_PIXEL_BYTES: f64 = 2.0;
const STROKE_SEGMENT_BYTES: f64 = 1280.0;
const STROKE_LINE_BYTES: f64 = 64.0;
const STROKE_PIXEL_BYTES: f64 = 4.0;
/// Segments a dash adds to a stroke's outline: its own and its caps.
const DASH_SEGMENTS: f64 = 2.0;
/// Widths of outline a stroked segment adds (its sides and its join).
const STROKE_WIDTHS: f64 = 3.0;

/// What a path costs to flatten and dash, in device pixels under a
/// transform, from its control polygons (upper bounds).
struct Extent {
    segments: f64,
    /// Segments with a part within the pixmap.
    near: f64,
    /// Further lines its curves flatten into, within the pixmap.
    lines: f64,
    /// Its length within the pixmap.
    visible: f64,
    /// Its whole length.
    length: f64,
}

/// Tandem patch: the extent of `path` under `transform` in a pixmap of
/// `area`.
fn path_extent(path: &BezPath, transform: &Affine, area: Rect) -> Extent {
    let (mut segments, mut near, mut lines, mut visible, mut length) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let mut last = Point::ZERO;
    let mut start = Point::ZERO;
    // A segment's whole length and the most of it inside `area` (the
    // diagonal of its control points' box there).
    let polygon = |points: &[Point], from: Point| {
        let mut previous = *transform * from;
        let mut bounds = Rect::from_points(previous, previous);
        let mut total = 0.0;
        for point in points {
            let point = *transform * *point;
            total += (point - previous).hypot();
            bounds = bounds.union_pt(point);
            previous = point;
        }
        let inside = bounds.intersect(area);
        let seen = if inside.width() < 0.0 || inside.height() < 0.0 {
            0.0
        } else {
            inside.width().hypot(inside.height())
        };
        (total, seen.min(total))
    };
    for element in path.elements() {
        let (points, curve): (&[Point], bool) = match element {
            PathEl::MoveTo(p) => {
                (last, start) = (*p, *p);
                segments += 1.0;
                continue;
            }
            PathEl::ClosePath => (std::slice::from_ref(&start), false),
            PathEl::LineTo(p) => (std::slice::from_ref(p), false),
            PathEl::QuadTo(a, b) => (&[*a, *b][..], true),
            PathEl::CurveTo(a, b, c) => (&[*a, *b, *c][..], true),
        };
        let (reach, seen) = polygon(points, last);
        segments += 1.0;
        if seen > 0.0 {
            near += 1.0;
        }
        if curve {
            lines += seen.sqrt();
        }
        visible += seen;
        length += reach;
        last = points.last().copied().unwrap_or(last);
    }
    Extent {
        segments,
        near,
        lines,
        visible,
        length,
    }
}

impl Renderer<'_> {
    pub(super) fn set_stroke_properties(&mut self, stroke_props: &StrokeProps, is_text: bool) {
        let threshold = if is_text { 0.25 } else { 1.0 };

        // Best-effort attempt to ensure a line width of at least 1.0, as required by the PDF
        // specification. If we are stroking text, we reduce the threshold as it will otherwise
        // lead to very bold-looking text at low resolutions.
        let min_factor = max_factor(self.ctx.transform());
        let mut line_width = stroke_props.line_width.max(0.01 / min_factor);
        let transformed_width = line_width * min_factor;

        // Only enforce line width if not inside of pattern or type 3 glyph.
        if transformed_width < threshold && !self.inside_pattern && !self.in_type3_glyph {
            line_width /= transformed_width;
            line_width *= threshold;
        }

        let stroke = kurbo::Stroke {
            width: line_width as f64,
            join: stroke_props.line_join,
            miter_limit: stroke_props.miter_limit as f64,
            start_cap: stroke_props.line_cap,
            end_cap: stroke_props.line_cap,
            dash_pattern: stroke_props.dash_array.iter().map(|n| *n as f64).collect(),
            dash_offset: stroke_props.dash_offset as f64,
        };
        // Tandem patch: a dash pattern shorter than half a pixel (or 0.01
        // units) strokes solid, as MuPDF 1.28.1 does; the dashes kurbo
        // expanded for it cost memory for nothing visible.
        let period: f64 = stroke.dash_pattern.iter().sum();
        let stroke = if !stroke.dash_pattern.is_empty()
            && (period < 0.01 || period * f64::from(min_factor) < 0.5)
        {
            kurbo::Stroke {
                dash_pattern: Default::default(),
                ..stroke
            }
        } else {
            stroke
        };

        self.ctx.set_stroke(stroke);
    }

    /// Tandem patch: whether drawing `path` (stroked with the current stroke
    /// when `stroke`) fits [`MAX_PATH_BYTES`]; else the render is over
    /// budget.
    pub(crate) fn path_fits(&self, path: &BezPath, stroke: bool) -> bool {
        let transform = *self.ctx.transform();
        let (width, height) = (f64::from(self.ctx.width()), f64::from(self.ctx.height()));
        let area = Rect::new(0.0, 0.0, width, height);
        let bytes = if stroke {
            // Segments whose outline reaches into the pixmap count.
            let margin = self.stroke_margin().min(width + height);
            let extent = path_extent(path, &transform, area.inflate(margin, margin));
            let style = self.ctx.stroke();
            let period: f64 = style.dash_pattern.iter().sum();
            let scale = f64::from(max_factor(&transform));
            let dashes = if style.dash_pattern.is_empty() || period <= 0.0 {
                0.0
            } else {
                extent.length / (period * scale) * style.dash_pattern.len() as f64
            };
            let outline = (style.width * scale * STROKE_WIDTHS).min(2.0 * (width + height));
            (extent.segments + dashes * DASH_SEGMENTS) * STROKE_SEGMENT_BYTES
                + extent.lines * STROKE_LINE_BYTES
                + (extent.visible + extent.near * outline) * STROKE_PIXEL_BYTES
        } else {
            let extent = path_extent(path, &transform, area);
            extent.segments * FILL_SEGMENT_BYTES
                + extent.lines * FILL_LINE_BYTES
                + extent.visible * FILL_PIXEL_BYTES
        };
        // Non-finite geometry counts as past the budget.
        if bytes <= MAX_PATH_BYTES {
            true
        } else {
            self.global.over_budget.set(true);
            false
        }
    }

    pub(super) fn stroke_path(
        &mut self,
        path: &BezPath,
        props: DrawProps<'_>,
        stroke_props: &StrokeProps,
        is_text: bool,
    ) {
        self.apply_draw_props(&props);
        self.set_stroke_properties(stroke_props, is_text);
        // Tandem patch: within the render's scene and path budgets.
        if !self.scene_fits(path.bounding_box(), self.stroke_margin())
            || !self.path_fits(path, true)
        {
            return;
        }

        let clip_path = self.set_paint(&props.paint, || path.bounding_box(), true);
        if let Some(clip_path) = clip_path.as_ref() {
            self.push_clip_path_inner(clip_path, FillRule::NonZero);
        }
        self.ctx.stroke_path(path);
        if clip_path.is_some() {
            self.ctx.pop_clip();
        }
    }

    pub(super) fn fill_path(&mut self, path: &BezPath, props: DrawProps<'_>, fill_rule: FillRule) {
        self.ctx.set_fill_rule(convert_fill_rule(fill_rule));
        self.apply_draw_props(&props);
        // Tandem patch: within the render's scene and path budgets.
        let bbox = path.bounding_box();
        if !self.scene_fits(bbox, 0.0) || !self.path_fits(path, false) {
            return;
        }

        let clip_path = self.set_paint(&props.paint, || bbox, false);
        if let Some(clip_path) = clip_path.as_ref() {
            self.push_clip_path_inner(clip_path, fill_rule);
        }

        self.ctx.fill_path(path);

        if clip_path.is_some() {
            self.ctx.pop_clip();
        }
    }

    pub(super) fn draw_path<'a>(
        &mut self,
        path: &BezPath,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    ) {
        match draw_mode {
            DrawMode::Fill(f) => {
                Self::fill_path(self, path, props, *f);
            }
            DrawMode::Stroke(s) => {
                Self::stroke_path(self, path, props, s, false);
            }
            DrawMode::FillAndStroke(f, s) => {
                Self::fill_path(self, path, props.clone(), *f);
                Self::stroke_path(self, path, props, s, false);
            }
            DrawMode::Invisible => {}
        }
    }

    pub(super) fn draw_rect<'a>(
        &mut self,
        rect: &Rect,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    ) {
        match draw_mode {
            DrawMode::Fill(fill_rule) => {
                self.ctx.set_fill_rule(convert_fill_rule(*fill_rule));
                self.apply_draw_props(&props);
                // Tandem patch: within the render's scene budget.
                if !self.scene_fits(*rect, 0.0) {
                    return;
                }

                let clip_path = self.set_paint(&props.paint, || *rect, false);
                if let Some(clip_path) = clip_path.as_ref() {
                    self.push_clip_path_inner(clip_path, *fill_rule);
                }

                self.ctx.fill_rect(rect);

                if clip_path.is_some() {
                    self.ctx.pop_clip();
                }
            }
            DrawMode::Stroke(s) => {
                let path = rect.to_path(0.1);
                Self::stroke_path(self, &path, props, s, false);
            }
            DrawMode::FillAndStroke(fill_rule, stroke_props) => {
                self.draw_rect(rect, props.clone(), &DrawMode::Fill(*fill_rule));
                let path = rect.to_path(0.1);
                Self::stroke_path(self, &path, props, stroke_props, false);
            }
            DrawMode::Invisible => {}
        }
    }
}

impl Renderer<'_> {
    /// Tandem patch: how far the current stroke reaches past its path, in
    /// pixels.
    pub(crate) fn stroke_margin(&self) -> f64 {
        let stroke = self.ctx.stroke();
        let reach = stroke.width * stroke.miter_limit.max(1.0);
        reach * f64::from(max_factor(self.ctx.transform()))
    }
}

pub(crate) fn max_factor(transform: &Affine) -> f32 {
    let scale_skew_transform = {
        let c = transform.as_coeffs();
        Affine::new([c[0], c[1], c[2], c[3], 0.0, 0.0])
    };

    let x_advance = scale_skew_transform * Point::new(1.0, 0.0);
    let y_advance = scale_skew_transform * Point::new(0.0, 1.0);

    x_advance
        .to_vec2()
        .length()
        .max(y_advance.to_vec2().length()) as f32
}
