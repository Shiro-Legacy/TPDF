// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded page rasterization by explicit 4x4 sample composition.
//!
//! Paths are flattened in device space to a 1/64-pixel control-hull tolerance,
//! then quantized to 1/256 pixel (nearest, half ties away from zero). Initial
//! page bounds are classified directly at sample centers, without quantization.
//! All paint/clip work uses the independent metered rational scanner. There is
//! no tiny-skia dependency, CMYK conversion, transparency, or hinting.
//! Solid positive-width stroke geometry uses the separately metered stroke stage.
//! The input's G1 and document allocations are separately bounded prerequisites.
use crate::geometry::{Affine, DeviceSpace, GeometryError, PageSpace, Point, UserSpace};
use crate::image_sampling::{
    ImagePlacement, ImageSamplingError, ImageSamplingLimits, ImageSamplingSession,
    ImageSamplingUsage,
};
use crate::page_glyph::{PageGlyphCommand, PageGlyphProgram};
use crate::path::{DeviceColor, FillRule, InitialClip, PathCommand, PathEvent};
use crate::program::ProgramEventKind;
use crate::scan_raster::{ScanEdge, ScanError, ScanLimits, ScanPoint, ScanSession, ScanUsage};
use crate::stroke::{self, StrokeError, StrokeLimits, StrokeSession, StrokeUsage};
use crate::text_position::TextEvent;
use std::{fmt, mem::size_of, ops::Range};
const SAMPLES: u32 = 4;
const MAX_Q: f64 = (1u32 << 29) as f64;
const FLATNESS_SQUARED: f64 = 1. / (64. * 64.);
#[derive(Clone, Copy, Debug)]
pub struct RasterTarget {
    pub width: u32,
    pub height: u32,
    pub page_to_device: Affine<PageSpace, DeviceSpace>,
}
#[derive(Clone, Copy, Debug)]
pub struct PageRasterLimits {
    /// Cumulative stroke expansion cap for the whole page, before pixel allocation.
    pub stroke: StrokeLimits,
    pub images: ImageSamplingLimits,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: usize,
    pub max_events: usize,
    pub max_source_commands: usize,
    pub max_flat_edges: usize,
    pub max_curve_nodes: usize,
    pub max_subdivision_depth: usize,
    pub max_clip_depth: usize,
    pub max_mask_bytes: usize,
    pub max_sample_bytes: usize,
    pub max_output_bytes: usize,
    pub max_allocated_bytes: usize,
    pub max_work: u64,
}
impl Default for PageRasterLimits {
    fn default() -> Self {
        Self {
            stroke: StrokeLimits::default(),
            images: ImageSamplingLimits::default(),
            max_width: 4096,
            max_height: 4096,
            max_pixels: 4_194_304,
            max_events: 100_000,
            max_source_commands: 1_000_000,
            max_flat_edges: 2_000_000,
            max_curve_nodes: 4_000_000,
            max_subdivision_depth: 24,
            max_clip_depth: 128,
            max_mask_bytes: 256 << 20,
            max_sample_bytes: 256 << 20,
            max_output_bytes: 16 << 20,
            max_allocated_bytes: 1 << 30,
            max_work: 2_000_000_000,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PageRasterSessionLimits {
    /// Cumulative stroke expansion cap across successful and failed pages.
    pub stroke: StrokeLimits,
    pub images: ImageSamplingLimits,
    pub raster: PageRasterLimits,
    pub scan: ScanLimits,
    pub max_page_attempts: usize,
}
impl Default for PageRasterSessionLimits {
    fn default() -> Self {
        Self {
            images: ImageSamplingLimits {
                max_preparations: 1_000_000,
                max_samples: 256_000_000,
                max_work: 4_300_000_000_000,
            },
            stroke: StrokeLimits {
                max_attempts: 1_000_000,
                max_input_commands: 16_000_000,
                max_subpaths: 1_000_000,
                max_line_segments: 16_000_000,
                max_curve_nodes: 64_000_000,
                max_curve_leaves: 32_000_000,
                max_join_attempts: 16_000_000,
                max_cap_attempts: 16_000_000,
                max_arc_nodes: 64_000_000,
                max_arc_vertices: 32_000_000,
                max_components: 32_000_000,
                max_output_commands: 160_000_000,
                max_output_points: 128_000_000,
                max_allocated_bytes: 2 << 30,
                max_work: 4_000_000_000,
                ..StrokeLimits::default()
            },
            raster: PageRasterLimits {
                max_pixels: 64_000_000,
                max_events: 10_000_000,
                max_source_commands: 16_000_000,
                max_flat_edges: 32_000_000,
                max_curve_nodes: 64_000_000,
                max_mask_bytes: 1 << 30,
                max_sample_bytes: 1 << 30,
                max_output_bytes: 256 << 20,
                max_allocated_bytes: 2 << 30,
                max_work: 16_000_000_000,
                ..PageRasterLimits::default()
            },
            scan: ScanLimits::default(),
            max_page_attempts: 10_000,
        }
    }
}
/// Cumulative admitted work, including reservations retained on failed renders.
/// Mask, sample, and output bytes reserve planned buffers before semantic checks;
/// `allocated_bytes` counts full allocation requests actually attempted.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PageRasterUsage {
    pub page_attempts: usize,
    pub pixels: usize,
    pub events: usize,
    pub source_commands: usize,
    pub flat_edges: usize,
    pub curve_nodes: usize,
    pub mask_bytes: usize,
    pub sample_bytes: usize,
    pub output_bytes: usize,
    pub allocated_bytes: usize,
    pub work: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PageRasterErrorKind {
    Unsupported(&'static str),
    InvalidTarget(&'static str),
    Limit(&'static str),
    Geometry(GeometryError),
    Scan(ScanError),
    Stroke(StrokeError),
    ImageSampling(ImageSamplingError),
    Allocation,
    Invariant(&'static str),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageRasterError {
    pub event_index: Option<usize>,
    pub operation_index: Option<usize>,
    pub span: Option<Range<usize>>,
    pub kind: PageRasterErrorKind,
}
impl fmt::Display for PageRasterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "page raster event {:?}, operation {:?} at {:?}: {:?}",
            self.event_index, self.operation_index, self.span, self.kind
        )
    }
}
impl std::error::Error for PageRasterError {}
#[derive(Clone, Debug, Default)]
struct Location {
    event: Option<usize>,
    op: Option<usize>,
    span: Option<Range<usize>>,
}
fn error(loc: &Location, kind: PageRasterErrorKind) -> PageRasterError {
    PageRasterError {
        event_index: loc.event,
        operation_index: loc.op,
        span: loc.span.clone(),
        kind,
    }
}
fn add(
    value: &mut usize,
    n: usize,
    limit: usize,
    loc: &Location,
    what: &'static str,
) -> Result<(), PageRasterError> {
    *value = value
        .checked_add(n)
        .filter(|v| *v <= limit)
        .ok_or_else(|| error(loc, PageRasterErrorKind::Limit(what)))?;
    Ok(())
}
fn mul(a: usize, b: usize, loc: &Location) -> Result<usize, PageRasterError> {
    a.checked_mul(b)
        .ok_or_else(|| error(loc, PageRasterErrorKind::Limit("size arithmetic")))
}
struct Budget {
    limits: PageRasterLimits,
    usage: PageRasterUsage,
}
impl Budget {
    fn work(&mut self, n: u64, l: &Location) -> Result<(), PageRasterError> {
        self.usage.work = self
            .usage
            .work
            .checked_add(n)
            .filter(|v| *v <= self.limits.max_work)
            .ok_or_else(|| error(l, PageRasterErrorKind::Limit("work")))?;
        Ok(())
    }
    fn allocation(&mut self, n: usize, l: &Location) -> Result<(), PageRasterError> {
        add(
            &mut self.usage.allocated_bytes,
            n,
            self.limits.max_allocated_bytes,
            l,
            "requested allocation",
        )
    }
    fn reserve<T>(
        &mut self,
        v: &mut Vec<T>,
        additional: usize,
        max: usize,
        l: &Location,
    ) -> Result<(), PageRasterError> {
        let end = v
            .len()
            .checked_add(additional)
            .filter(|v| *v <= max)
            .ok_or_else(|| error(l, PageRasterErrorKind::Limit("arena entries")))?;
        if end <= v.capacity() {
            return Ok(());
        }
        let cap = v.capacity().saturating_mul(2).min(max).max(end);
        self.allocation(mul(cap, size_of::<T>(), l)?, l)?;
        v.try_reserve_exact(cap - v.len())
            .map_err(|_| error(l, PageRasterErrorKind::Allocation))
    }
    fn buffer(&mut self, n: usize, value: u8, l: &Location) -> Result<Vec<u8>, PageRasterError> {
        self.allocation(n, l)?;
        self.work(n as u64, l)?;
        let mut v = Vec::new();
        v.try_reserve_exact(n)
            .map_err(|_| error(l, PageRasterErrorKind::Allocation))?;
        v.resize(n, value);
        Ok(v)
    }
}
#[derive(Clone, Copy)]
struct P {
    x: f64,
    y: f64,
}
fn mapped<S>(p: Point<S>, map: Affine<S, DeviceSpace>, l: &Location) -> Result<P, PageRasterError> {
    let p = map
        .transform_point(p)
        .map_err(|e| error(l, PageRasterErrorKind::Geometry(e)))?;
    if p.x().abs() > MAX_Q / 256. || p.y().abs() > MAX_Q / 256. {
        return Err(error(
            l,
            PageRasterErrorKind::Limit("device coordinate magnitude"),
        ));
    }
    Ok(P { x: p.x(), y: p.y() })
}
fn quantize(p: P, l: &Location) -> Result<ScanPoint, PageRasterError> {
    let x = (p.x * 256.).round();
    let y = (p.y * 256.).round();
    if !x.is_finite() || !y.is_finite() || x.abs() > MAX_Q || y.abs() > MAX_Q {
        return Err(error(l, PageRasterErrorKind::Limit("quantized coordinate")));
    }
    Ok(ScanPoint {
        x: x as i32,
        y: y as i32,
    })
}
#[derive(Clone, Copy)]
struct Curve {
    p: [P; 4],
    degree: usize,
    depth: usize,
}
fn midpoint(a: P, b: P) -> P {
    P {
        x: (a.x + b.x) * 0.5,
        y: (a.y + b.y) * 0.5,
    }
}
fn flat(curve: Curve) -> bool {
    let a = curve.p[0];
    let z = curve.p[curve.degree];
    let dx = z.x - a.x;
    let dy = z.y - a.y;
    let length = dx * dx + dy * dy;
    curve.p[1..curve.degree].iter().all(|p| {
        let t = if length == 0. {
            0.
        } else {
            ((p.x - a.x) * dx + (p.y - a.y) * dy) / length
        }
        .clamp(0., 1.);
        let x = p.x - (a.x + t * dx);
        let y = p.y - (a.y + t * dy);
        x * x + y * y <= FLATNESS_SQUARED
    })
}
#[derive(Default)]
struct FlatPath {
    edges: Vec<ScanEdge>,
}
struct Flattener<'a> {
    out: FlatPath,
    first: Option<P>,
    last: Option<P>,
    budget: &'a mut Budget,
    loc: &'a Location,
}
impl<'a> Flattener<'a> {
    fn new(budget: &'a mut Budget, loc: &'a Location) -> Self {
        Self {
            out: FlatPath::default(),
            first: None,
            last: None,
            budget,
            loc,
        }
    }
    fn edge(&mut self, a: P, z: P) -> Result<(), PageRasterError> {
        self.budget.work(1, self.loc)?;
        let a = quantize(a, self.loc)?;
        let z = quantize(z, self.loc)?;
        if a.x == z.x && a.y == z.y {
            return Ok(());
        }
        add(
            &mut self.budget.usage.flat_edges,
            1,
            self.budget.limits.max_flat_edges,
            self.loc,
            "flattened edges",
        )?;
        self.budget.reserve(
            &mut self.out.edges,
            1,
            self.budget.limits.max_flat_edges,
            self.loc,
        )?;
        self.out.edges.push(ScanEdge { start: a, end: z });
        Ok(())
    }
    fn close(&mut self) -> Result<(), PageRasterError> {
        if let (Some(a), Some(z)) = (self.last, self.first) {
            self.edge(a, z)?;
            self.last = Some(z);
        }
        Ok(())
    }
    fn moveto(&mut self, p: P) -> Result<(), PageRasterError> {
        self.close()?;
        self.first = Some(p);
        self.last = Some(p);
        Ok(())
    }
    fn lineto(&mut self, p: P) -> Result<(), PageRasterError> {
        let last = self.last.ok_or_else(|| {
            error(
                self.loc,
                PageRasterErrorKind::Invariant("line without current point"),
            )
        })?;
        self.edge(last, p)?;
        self.last = Some(p);
        Ok(())
    }
    fn curve(&mut self, p: [P; 4], degree: usize) -> Result<(), PageRasterError> {
        let mut stack = [None; 25];
        let mut count = 1;
        stack[0] = Some(Curve {
            p,
            degree,
            depth: 0,
        });
        while count > 0 {
            count -= 1;
            let curve = stack[count].take().expect("curve stack");
            add(
                &mut self.budget.usage.curve_nodes,
                1,
                self.budget.limits.max_curve_nodes,
                self.loc,
                "curve nodes",
            )?;
            self.budget.work(4, self.loc)?;
            if flat(curve) {
                self.lineto(curve.p[degree])?;
                continue;
            }
            if curve.depth >= self.budget.limits.max_subdivision_depth.min(24) {
                return Err(error(
                    self.loc,
                    PageRasterErrorKind::Limit("curve subdivision depth"),
                ));
            }
            let a = midpoint(curve.p[0], curve.p[1]);
            let b = midpoint(curve.p[1], curve.p[2]);
            let (left, right) = if degree == 2 {
                let m = midpoint(a, b);
                ([curve.p[0], a, m, m], [m, b, curve.p[2], curve.p[2]])
            } else {
                let c = midpoint(curve.p[2], curve.p[3]);
                let d = midpoint(a, b);
                let e = midpoint(b, c);
                let m = midpoint(d, e);
                ([curve.p[0], a, d, m], [m, e, c, curve.p[3]])
            };
            stack[count] = Some(Curve {
                p: right,
                degree,
                depth: curve.depth + 1,
            });
            stack[count + 1] = Some(Curve {
                p: left,
                degree,
                depth: curve.depth + 1,
            });
            count += 2;
        }
        Ok(())
    }
    fn finish(mut self) -> Result<FlatPath, PageRasterError> {
        self.close()?;
        Ok(self.out)
    }
}
fn user_path(
    commands: &[PathCommand],
    map: Affine<UserSpace, DeviceSpace>,
    b: &mut Budget,
    l: &Location,
) -> Result<FlatPath, PageRasterError> {
    add(
        &mut b.usage.source_commands,
        commands.len(),
        b.limits.max_source_commands,
        l,
        "source commands",
    )?;
    b.work(commands.len() as u64 * 4, l)?;
    let mut f = Flattener::new(b, l);
    for command in commands {
        match *command {
            PathCommand::MoveTo(p) => f.moveto(mapped(p, map, l)?)?,
            PathCommand::LineTo(p) => f.lineto(mapped(p, map, l)?)?,
            PathCommand::ClosePath => f.close()?,
            PathCommand::CubicTo {
                control1,
                control2,
                end,
            } => {
                let a = f.last.ok_or_else(|| {
                    error(
                        l,
                        PageRasterErrorKind::Invariant("curve without current point"),
                    )
                })?;
                f.curve(
                    [
                        a,
                        mapped(control1, map, l)?,
                        mapped(control2, map, l)?,
                        mapped(end, map, l)?,
                    ],
                    3,
                )?;
            }
        }
    }
    f.finish()
}
fn glyph_path(
    commands: &[PageGlyphCommand],
    map: Affine<PageSpace, DeviceSpace>,
    b: &mut Budget,
    l: &Location,
) -> Result<FlatPath, PageRasterError> {
    add(
        &mut b.usage.source_commands,
        commands.len(),
        b.limits.max_source_commands,
        l,
        "source commands",
    )?;
    b.work(commands.len() as u64 * 4, l)?;
    let mut f = Flattener::new(b, l);
    for command in commands {
        match *command {
            PageGlyphCommand::MoveTo(p) => f.moveto(mapped(p, map, l)?)?,
            PageGlyphCommand::LineTo(p) => f.lineto(mapped(p, map, l)?)?,
            PageGlyphCommand::Close => f.close()?,
            PageGlyphCommand::CubicTo {
                control1,
                control2,
                end,
            } => {
                let a = f.last.ok_or_else(|| {
                    error(
                        l,
                        PageRasterErrorKind::Invariant("glyph curve without current point"),
                    )
                })?;
                f.curve(
                    [
                        a,
                        mapped(control1, map, l)?,
                        mapped(control2, map, l)?,
                        mapped(end, map, l)?,
                    ],
                    3,
                )?;
            }
            PageGlyphCommand::QuadTo { control, end } => {
                let a = f.last.ok_or_else(|| {
                    error(
                        l,
                        PageRasterErrorKind::Invariant("glyph curve without current point"),
                    )
                })?;
                let z = mapped(end, map, l)?;
                f.curve([a, mapped(control, map, l)?, z, z], 2)?;
            }
        }
    }
    f.finish()
}
fn color(color: DeviceColor, l: &Location) -> Result<[u8; 3], PageRasterError> {
    let bytes = |v: f64| ((v * 255.).round().clamp(0., 255.)) as u8;
    match color {
        DeviceColor::Gray(v) => Ok([bytes(v); 3]),
        DeviceColor::Rgb([r, g, b]) => Ok([bytes(r), bytes(g), bytes(b)]),
        DeviceColor::Cmyk(..) => Err(error(l, PageRasterErrorKind::Unsupported("CMYK paint"))),
    }
}
struct PlannedClip {
    path: FlatPath,
    /// Sample-grid rectangle [x0, y0, x1) x [y0, y1) holding every covered
    /// sample of this clip and its ancestors; outside it the mask is zero.
    bounds: [u32; 4],
    parent: Option<usize>,
    rule: FillRule,
    depth: usize,
    loc: Location,
}
struct PlannedImage {
    placement: ImagePlacement,
    asset: usize,
    bounds: [u32; 4],
}
struct PlannedPaint {
    image: Option<usize>,
    path: FlatPath,
    clip: Option<usize>,
    rule: FillRule,
    color: [u8; 3],
    loc: Location,
}
struct Plan {
    clips: Vec<PlannedClip>,
    paints: Vec<PlannedPaint>,
    images: Vec<PlannedImage>,
    bounds: [f64; 4],
    width: u32,
    height: u32,
    sample_width: u32,
    sample_height: u32,
    sample_count: usize,
}
fn prepare(
    input: &PageGlyphProgram,
    target: RasterTarget,
    b: &mut Budget,
    strokes: &mut StrokeSession,
    images: &mut ImageSamplingSession,
) -> Result<Plan, PageRasterError> {
    let base = Location::default();
    let [a, bb, c, d, _, _] = target.page_to_device.coefficients();
    if a <= 0. || a != d || bb != 0. || c != 0. {
        return Err(error(
            &base,
            PageRasterErrorKind::InvalidTarget("positive uniform scale and translation required"),
        ));
    }
    if target.width == 0
        || target.height == 0
        || target.width > b.limits.max_width
        || target.height > b.limits.max_height
        || target.width > (1 << 21)
        || target.height > (1 << 21)
    {
        return Err(error(
            &base,
            PageRasterErrorKind::InvalidTarget("canvas dimensions"),
        ));
    }
    let pixels = mul(target.width as usize, target.height as usize, &base)?;
    add(
        &mut b.usage.pixels,
        pixels,
        b.limits.max_pixels,
        &base,
        "output pixels",
    )?;
    let sample_count = mul(pixels, 16, &base)?;
    let sample_bytes = mul(sample_count, 4, &base)?;
    let output_bytes = mul(pixels, 4, &base)?;
    let page = input.page_program();
    let program = page.program();
    let paths = program.paths();
    add(
        &mut b.usage.events,
        program.events().len(),
        b.limits.max_events,
        &base,
        "events",
    )?;
    b.work(program.events().len() as u64, &base)?;
    let InitialClip::Rectangle(initial) = paths.initial_clip() else {
        return Err(error(
            &base,
            PageRasterErrorKind::Invariant("PP1 initial page rectangle"),
        ));
    };
    let bounds = target
        .page_to_device
        .transform_bounds(initial)
        .map_err(|e| error(&base, PageRasterErrorKind::Geometry(e)))?
        .bounds();
    if bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
        return Err(error(
            &base,
            PageRasterErrorKind::InvalidTarget("page bounds underflow"),
        ));
    }
    // Translation must not silently round away part of the original page extent.
    // This checks precision only; accepted bounds remain unsnapped for samples.
    let expected_width = initial.width() * a;
    let expected_height = initial.height() * a;
    if !expected_width.is_finite()
        || !expected_height.is_finite()
        || expected_width <= 0.
        || expected_height <= 0.
        || ((bounds[2] - bounds[0]) - expected_width).abs() > expected_width * (64. * f64::EPSILON)
        || ((bounds[3] - bounds[1]) - expected_height).abs()
            > expected_height * (64. * f64::EPSILON)
    {
        return Err(error(
            &base,
            PageRasterErrorKind::InvalidTarget("page transform loses extent precision"),
        ));
    }
    // The page mask is full size; each clip later charges only its bounds.
    add(
        &mut b.usage.mask_bytes,
        sample_count,
        b.limits.max_mask_bytes,
        &base,
        "mask bytes",
    )?;
    add(
        &mut b.usage.sample_bytes,
        sample_bytes,
        b.limits.max_sample_bytes,
        &base,
        "sample bytes",
    )?;
    add(
        &mut b.usage.output_bytes,
        output_bytes,
        b.limits.max_output_bytes,
        &base,
        "output bytes",
    )?;
    // Full-page unsupported semantic checks precede geometry and all pixels.
    for (event_index, event) in program.events().iter().enumerate() {
        let loc = Location {
            event: Some(event_index),
            op: Some(event.operation_index()),
            span: Some(event.span()),
        };
        match event.kind() {
            ProgramEventKind::Image(index) => {
                let paint = program.images().get(index).ok_or_else(|| {
                    error(&loc, PageRasterErrorKind::Invariant("image event identity"))
                })?;
                if paint.operation_index() != event.operation_index()
                    || page.images().get(paint.image_id()).is_none()
                {
                    return Err(error(
                        &loc,
                        PageRasterErrorKind::Invariant("image asset provenance"),
                    ));
                }
            }
            ProgramEventKind::Path(index) => match &paths.events()[index] {
                PathEvent::Stroke { paint, style } => {
                    b.work(8, &loc)?;
                    color(paint.color(), &loc)?;
                    let dash = style
                        .dash()
                        .map(|id| {
                            paths.dashes().get(id.index()).ok_or_else(|| {
                                error(&loc, PageRasterErrorKind::Invariant("stroke dash identity"))
                            })
                        })
                        .transpose()?;
                    stroke::validate_style(*style, dash)
                        .map_err(|e| error(&loc, PageRasterErrorKind::Stroke(e)))?;
                }
                PathEvent::Fill { paint, .. } => {
                    color(paint.color(), &loc)?;
                }
                _ => {}
            },
            ProgramEventKind::Text {
                event: index,
                context,
            } => {
                if let TextEvent::Code(index) = program.text().events()[index] {
                    if program.text().codes()[index].rendering_mode() == 0 {
                        color(program.text_contexts()[context].fill_color(), &loc)?;
                    }
                }
            }
        }
    }
    let mut plan = Plan {
        clips: Vec::new(),
        paints: Vec::new(),
        images: Vec::new(),
        bounds,
        width: target.width,
        height: target.height,
        sample_width: target.width * 4,
        sample_height: target.height * 4,
        sample_count,
    };
    let mut glyph_index = 0;
    for (event_index, event) in program.events().iter().enumerate() {
        let loc = Location {
            event: Some(event_index),
            op: Some(event.operation_index()),
            span: Some(event.span()),
        };
        b.work(1, &loc)?;
        match event.kind() {
            ProgramEventKind::Image(index) => {
                let paint = program.images()[index];
                let asset = &page.images()[paint.image_id()];
                let transform = paint
                    .ctm()
                    .then(target.page_to_device)
                    .map_err(|e| error(&loc, PageRasterErrorKind::Geometry(e)))?;
                let placement = images
                    .prepare(transform, asset.width() as u32, asset.height() as u32)
                    .map_err(|e| error(&loc, PageRasterErrorKind::ImageSampling(e)))?;
                let bounds = placement.sample_bounds(plan.sample_width, plan.sample_height);
                let clip = paint.clip().map(|v| v.index());
                if clip.is_some_and(|id| id >= plan.clips.len()) {
                    return Err(error(
                        &loc,
                        PageRasterErrorKind::Invariant("image clip identity"),
                    ));
                }
                b.reserve(&mut plan.images, 1, b.limits.max_events, &loc)?;
                b.reserve(&mut plan.paints, 1, b.limits.max_events, &loc)?;
                let index = plan.images.len();
                plan.images.push(PlannedImage {
                    placement,
                    asset: paint.image_id(),
                    bounds,
                });
                plan.paints.push(PlannedPaint {
                    image: Some(index),
                    path: FlatPath { edges: Vec::new() },
                    clip,
                    rule: FillRule::NonZero,
                    color: [0; 3],
                    loc,
                });
            }
            ProgramEventKind::Path(index) => match &paths.events()[index] {
                PathEvent::Fill { paint, rule } => {
                    let transform = paint
                        .ctm()
                        .then(target.page_to_device)
                        .map_err(|e| error(&loc, PageRasterErrorKind::Geometry(e)))?;
                    let path = user_path(
                        paths.paths()[paint.path().index()].commands(),
                        transform,
                        b,
                        &loc,
                    )?;
                    b.reserve(&mut plan.paints, 1, b.limits.max_events, &loc)?;
                    plan.paints.push(PlannedPaint {
                        image: None,
                        path,
                        clip: paint.clip().map(|v| v.index()),
                        rule: *rule,
                        color: color(paint.color(), &loc)?,
                        loc,
                    });
                }
                PathEvent::ClipPush { id, .. } => {
                    if id.index() != plan.clips.len() {
                        return Err(error(&loc, PageRasterErrorKind::Invariant("clip order")));
                    }
                    let node = &paths.clips()[id.index()];
                    let parent = node.parent().map(|v| v.index());
                    let depth = if let Some(p) = parent {
                        plan.clips
                            .get(p)
                            .ok_or_else(|| {
                                error(&loc, PageRasterErrorKind::Invariant("clip parent"))
                            })?
                            .depth
                            + 1
                    } else {
                        1
                    };
                    if depth > b.limits.max_clip_depth {
                        return Err(error(&loc, PageRasterErrorKind::Limit("clip depth")));
                    }
                    let transform = node
                        .ctm()
                        .then(target.page_to_device)
                        .map_err(|e| error(&loc, PageRasterErrorKind::Geometry(e)))?;
                    let path = user_path(
                        paths.paths()[node.path().index()].commands(),
                        transform,
                        b,
                        &loc,
                    )?;
                    let mut bounds = [plan.sample_width, plan.sample_height, 0, 0];
                    for point in path.edges.iter().flat_map(|e| [e.start, e.end]) {
                        // 1/256-pixel lattice, four samples per pixel; one
                        // sample of slack on each side covers center rounding.
                        let x = i64::from(point.x).div_euclid(64);
                        let y = i64::from(point.y).div_euclid(64);
                        let clamp = |v: i64, max: u32| v.clamp(0, i64::from(max)) as u32;
                        bounds[0] = bounds[0].min(clamp(x - 1, plan.sample_width));
                        bounds[1] = bounds[1].min(clamp(y - 1, plan.sample_height));
                        bounds[2] = bounds[2].max(clamp(x + 2, plan.sample_width));
                        bounds[3] = bounds[3].max(clamp(y + 2, plan.sample_height));
                    }
                    if let Some(parent) = parent.map(|p| plan.clips[p].bounds) {
                        bounds[0] = bounds[0].max(parent[0]);
                        bounds[1] = bounds[1].max(parent[1]);
                        bounds[2] = bounds[2].min(parent[2]);
                        bounds[3] = bounds[3].min(parent[3]);
                    }
                    bounds[2] = bounds[2].max(bounds[0]);
                    bounds[3] = bounds[3].max(bounds[1]);
                    add(
                        &mut b.usage.mask_bytes,
                        mul(
                            (bounds[2] - bounds[0]) as usize,
                            (bounds[3] - bounds[1]) as usize,
                            &loc,
                        )?,
                        b.limits.max_mask_bytes,
                        &loc,
                        "mask bytes",
                    )?;
                    b.reserve(&mut plan.clips, 1, paths.clips().len(), &loc)?;
                    plan.clips.push(PlannedClip {
                        path,
                        bounds,
                        parent,
                        rule: node.rule(),
                        depth,
                        loc,
                    });
                }
                PathEvent::ClipRestore { .. } => {}
                PathEvent::Stroke { paint, style } => {
                    let transform = paint
                        .ctm()
                        .then(target.page_to_device)
                        .map_err(|e| error(&loc, PageRasterErrorKind::Geometry(e)))?;
                    let dash = style
                        .dash()
                        .map(|id| {
                            paths.dashes().get(id.index()).ok_or_else(|| {
                                error(&loc, PageRasterErrorKind::Invariant("stroke dash identity"))
                            })
                        })
                        .transpose()?;
                    let outline = strokes
                        .expand(
                            paths.paths()[paint.path().index()].commands(),
                            *style,
                            dash,
                            transform,
                            b.limits.stroke,
                        )
                        .map_err(|e| error(&loc, PageRasterErrorKind::Stroke(e)))?;
                    let path = user_path(outline.commands(), transform, b, &loc)?;
                    b.reserve(&mut plan.paints, 1, b.limits.max_events, &loc)?;
                    plan.paints.push(PlannedPaint {
                        image: None,
                        path,
                        clip: paint.clip().map(|v| v.index()),
                        rule: FillRule::NonZero,
                        color: color(paint.color(), &loc)?,
                        loc,
                    });
                }
            },
            ProgramEventKind::Text {
                event: text_event,
                context,
            } => {
                let TextEvent::Code(code_index) = program.text().events()[text_event] else {
                    continue;
                };
                let glyph = input.glyphs().get(glyph_index).ok_or_else(|| {
                    error(&loc, PageRasterErrorKind::Invariant("glyph attachment"))
                })?;
                glyph_index += 1;
                if glyph.event_index() != event_index
                    || glyph.code_index() != code_index
                    || glyph.context_id() != context
                {
                    return Err(error(
                        &loc,
                        PageRasterErrorKind::Invariant("glyph event provenance"),
                    ));
                }
                let code = &program.text().codes()[code_index];
                if code.rendering_mode() == 3 {
                    continue;
                }
                if code.rendering_mode() != 0 {
                    return Err(error(
                        &loc,
                        PageRasterErrorKind::Unsupported("text rendering mode"),
                    ));
                }
                let path = glyph_path(
                    &input.commands()[glyph.command_range()],
                    target.page_to_device,
                    b,
                    &loc,
                )?;
                b.reserve(&mut plan.paints, 1, b.limits.max_events, &loc)?;
                plan.paints.push(PlannedPaint {
                    image: None,
                    path,
                    clip: program.text_contexts()[context].clip().map(|v| v.index()),
                    rule: FillRule::NonZero,
                    color: color(program.text_contexts()[context].fill_color(), &loc)?,
                    loc,
                });
            }
        }
    }
    if glyph_index != input.glyphs().len() {
        return Err(error(
            &base,
            PageRasterErrorKind::Invariant("unused glyph attachments"),
        ));
    }
    Ok(plan)
}
#[derive(Debug, Eq, PartialEq)]
pub struct PageRasterImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
impl PageRasterImage {
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}
pub struct PageRasterSession {
    limits: PageRasterSessionLimits,
    usage: PageRasterUsage,
    scan: ScanSession,
    stroke_usage: StrokeUsage,
    image_usage: ImageSamplingUsage,
}
impl PageRasterSession {
    pub fn new(limits: PageRasterSessionLimits) -> Self {
        Self {
            limits,
            usage: PageRasterUsage::default(),
            scan: ScanSession::new(limits.scan),
            stroke_usage: StrokeUsage::default(),
            image_usage: ImageSamplingUsage::default(),
        }
    }
    pub fn usage(&self) -> PageRasterUsage {
        self.usage
    }
    pub fn image_usage(&self) -> ImageSamplingUsage {
        self.image_usage
    }
    pub fn stroke_usage(&self) -> StrokeUsage {
        self.stroke_usage
    }
    pub fn scan_usage(&self) -> ScanUsage {
        self.scan.usage()
    }
    pub fn render(
        &mut self,
        input: &PageGlyphProgram,
        target: RasterTarget,
        limits: PageRasterLimits,
    ) -> Result<PageRasterImage, PageRasterError> {
        add(
            &mut self.usage.page_attempts,
            1,
            self.limits.max_page_attempts,
            &Location::default(),
            "page attempts",
        )?;
        let limits = remaining(self.usage, self.limits.raster, limits);
        let stroke_cap = self
            .limits
            .stroke
            .remaining(self.stroke_usage)
            .map_err(|e| error(&Location::default(), PageRasterErrorKind::Stroke(e)))?;
        let mut strokes = StrokeSession::new(limits.stroke.intersect(stroke_cap));
        let image_cap = ImageSamplingLimits {
            max_preparations: limits
                .images
                .max_preparations
                .min(self.limits.images.max_preparations - self.image_usage.preparations),
            max_samples: limits
                .images
                .max_samples
                .min(self.limits.images.max_samples - self.image_usage.samples),
            max_work: limits
                .images
                .max_work
                .min(self.limits.images.max_work - self.image_usage.work),
        };
        let mut images = ImageSamplingSession::new(image_cap);
        let mut budget = Budget {
            limits,
            usage: PageRasterUsage::default(),
        };
        let result = (|| {
            let plan = prepare(input, target, &mut budget, &mut strokes, &mut images)?;
            execute(&plan, input, &mut self.scan, &mut images, &mut budget)
        })();
        accumulate(&mut self.usage, budget.usage);
        let used = images.usage();
        self.image_usage.preparations += used.preparations;
        self.image_usage.samples += used.samples;
        self.image_usage.work += used.work;
        self.stroke_usage
            .accumulate(strokes.usage())
            .map_err(|e| error(&Location::default(), PageRasterErrorKind::Stroke(e)))?;
        result
    }
}
/// Clip coverage stored only within its sample-grid bounds.
struct ClipMask {
    bounds: [u32; 4],
    data: Vec<u8>,
}
impl ClipMask {
    fn get(&self, x: u32, y: u32) -> u8 {
        let [x0, y0, x1, y1] = self.bounds;
        if x < x0 || x >= x1 || y < y0 || y >= y1 {
            return 0;
        }
        self.data[(y - y0) as usize * (x1 - x0) as usize + (x - x0) as usize]
    }
}
fn execute(
    plan: &Plan,
    input: &PageGlyphProgram,
    scan: &mut ScanSession,
    images: &mut ImageSamplingSession,
    b: &mut Budget,
) -> Result<PageRasterImage, PageRasterError> {
    let loc = Location::default();
    let mut base = b.buffer(plan.sample_count, 0, &loc)?;
    b.work(plan.sample_count as u64, &loc)?;
    for y in 0..plan.sample_height {
        let yy = (f64::from(y) + 0.5) / f64::from(SAMPLES);
        for x in 0..plan.sample_width {
            let xx = (f64::from(x) + 0.5) / f64::from(SAMPLES);
            if xx >= plan.bounds[0]
                && xx < plan.bounds[2]
                && yy >= plan.bounds[1]
                && yy < plan.bounds[3]
            {
                base[y as usize * plan.sample_width as usize + x as usize] = 255;
            }
        }
    }
    let base = ClipMask {
        bounds: [0, 0, plan.sample_width, plan.sample_height],
        data: base,
    };
    let mut masks: Vec<ClipMask> = Vec::new();
    b.reserve(&mut masks, plan.clips.len(), plan.clips.len(), &loc)?;
    for clip in &plan.clips {
        let [x0, y0, x1, y1] = clip.bounds;
        let width = (x1 - x0) as usize;
        let mut data = b.buffer(width * (y1 - y0) as usize, 0, &clip.loc)?;
        scan.scan(
            &clip.path.edges,
            clip.rule,
            plan.sample_width,
            plan.sample_height,
            |span| {
                if span.y < y0 || span.y >= y1 {
                    return;
                }
                let (from, to) = (span.x0.max(x0), span.x1.min(x1));
                if from < to {
                    let row = (span.y - y0) as usize * width;
                    data[row + (from - x0) as usize..row + (to - x0) as usize].fill(255);
                }
            },
        )
        .map_err(|e| error(&clip.loc, PageRasterErrorKind::Scan(e)))?;
        let parent = clip.parent.map(|id| &masks[id]).unwrap_or(&base);
        b.work(data.len() as u64, &clip.loc)?;
        for y in y0..y1 {
            for x in x0..x1 {
                data[(y - y0) as usize * width + (x - x0) as usize] &= parent.get(x, y);
            }
        }
        masks.push(ClipMask {
            bounds: clip.bounds,
            data,
        });
    }
    let mut samples = b.buffer(plan.sample_count * 4, 255, &loc)?;
    for paint in &plan.paints {
        let mask = paint.clip.map(|id| &masks[id]).unwrap_or(&base);
        if let Some(index) = paint.image {
            let image = &plan.images[index];
            let asset = &input.page_program().images()[image.asset];
            let [x0, y0, x1, y1] = image.bounds;
            b.work(u64::from(x1 - x0) * u64::from(y1 - y0), &paint.loc)?;
            for y in y0..y1 {
                for x in x0..x1 {
                    // Charge every candidate even when the current clip is empty.
                    let texel = images
                        .sample(&image.placement, x, y)
                        .map_err(|e| error(&paint.loc, PageRasterErrorKind::ImageSampling(e)))?;
                    let at = y as usize * plan.sample_width as usize + x as usize;
                    if mask.get(x, y) != 0 {
                        if let Some(texel) = texel {
                            let [r, g, bb, alpha] = asset
                                .pixel_rgba(texel.row() as usize, texel.column() as usize)
                                .ok_or_else(|| {
                                    error(
                                        &paint.loc,
                                        PageRasterErrorKind::Invariant("sampled image texel"),
                                    )
                                })?;
                            if alpha != 0 {
                                samples[at * 4..at * 4 + 4].copy_from_slice(&[r, g, bb, 255]);
                            }
                        }
                    }
                }
            }
            continue;
        }
        let [r, g, bb] = paint.color;
        scan.scan(
            &paint.path.edges,
            paint.rule,
            plan.sample_width,
            plan.sample_height,
            |span| {
                for x in span.x0..span.x1 {
                    let at = span.y as usize * plan.sample_width as usize + x as usize;
                    if mask.get(x, span.y) != 0 {
                        samples[at * 4..at * 4 + 4].copy_from_slice(&[r, g, bb, 255]);
                    }
                }
            },
        )
        .map_err(|e| error(&paint.loc, PageRasterErrorKind::Scan(e)))?;
    }
    let mut output = b.buffer(plan.width as usize * plan.height as usize * 4, 255, &loc)?;
    b.work(plan.sample_count as u64 * 4, &loc)?;
    for y in 0..plan.height {
        for x in 0..plan.width {
            let mut sum = [0u16; 3];
            for yy in 0..4 {
                for xx in 0..4 {
                    let at = ((y * 4 + yy) as usize * plan.sample_width as usize
                        + (x * 4 + xx) as usize)
                        * 4;
                    for c in 0..3 {
                        sum[c] += u16::from(samples[at + c]);
                    }
                }
            }
            let at = (y as usize * plan.width as usize + x as usize) * 4;
            for c in 0..3 {
                output[at + c] = ((sum[c] + 8) / 16) as u8;
            }
        }
    }
    Ok(PageRasterImage {
        width: plan.width,
        height: plan.height,
        pixels: output,
    })
}
fn remaining(
    used: PageRasterUsage,
    total: PageRasterLimits,
    mut cap: PageRasterLimits,
) -> PageRasterLimits {
    macro_rules! field {
        ($max:ident,$used:ident) => {
            cap.$max = cap.$max.min(total.$max - used.$used);
        };
    }
    field!(max_pixels, pixels);
    field!(max_events, events);
    field!(max_source_commands, source_commands);
    field!(max_flat_edges, flat_edges);
    field!(max_curve_nodes, curve_nodes);
    field!(max_mask_bytes, mask_bytes);
    field!(max_sample_bytes, sample_bytes);
    field!(max_output_bytes, output_bytes);
    field!(max_allocated_bytes, allocated_bytes);
    field!(max_work, work);
    cap.stroke = cap.stroke.intersect(total.stroke);
    cap.images.max_preparations = cap
        .images
        .max_preparations
        .min(total.images.max_preparations);
    cap.images.max_samples = cap.images.max_samples.min(total.images.max_samples);
    cap.images.max_work = cap.images.max_work.min(total.images.max_work);
    cap.max_width = cap.max_width.min(total.max_width);
    cap.max_height = cap.max_height.min(total.max_height);
    cap.max_subdivision_depth = cap.max_subdivision_depth.min(total.max_subdivision_depth);
    cap.max_clip_depth = cap.max_clip_depth.min(total.max_clip_depth);
    cap
}
fn accumulate(total: &mut PageRasterUsage, used: PageRasterUsage) {
    total.pixels += used.pixels;
    total.events += used.events;
    total.source_commands += used.source_commands;
    total.flat_edges += used.flat_edges;
    total.curve_nodes += used.curve_nodes;
    total.mask_bytes += used.mask_bytes;
    total.sample_bytes += used.sample_bytes;
    total.output_bytes += used.output_bytes;
    total.allocated_bytes += used.allocated_bytes;
    total.work += used.work;
}
