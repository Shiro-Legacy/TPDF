// SPDX-License-Identifier: AGPL-3.0-or-later
//! Optional bounded integer orthogonal rasterization through tiny-skia's safe
//! API. The backend has internal unsafe and infallible allocations; see
//! RASTER-DESIGN.md. This module does not promise recovery from process OOM.
use crate::geometry::{Affine, DeviceSpace, PageSpace, Point, UserSpace};
use crate::path::{DeviceColor, FillRule, InitialClip, PathCommand, PathEvent, PathProgram};
use std::{fmt, mem::size_of, ops::Range};

#[derive(Clone, Copy, Debug)]
pub struct RasterTarget {
    pub width: u32,
    pub height: u32,
    pub page_to_device: Affine<PageSpace, DeviceSpace>,
}
#[derive(Clone, Copy, Debug)]
pub struct RasterLimits {
    pub max_pixels: usize,
    pub max_output_bytes: usize,
    pub max_mask_bytes: usize,
    pub max_storage_bytes: usize,
    pub max_commands_per_path: usize,
    pub max_total_commands: usize,
    pub max_events: usize,
    pub max_clip_depth: usize,
    pub max_work: u64,
}
impl Default for RasterLimits {
    fn default() -> Self {
        Self {
            max_pixels: 4_194_304,
            max_output_bytes: 16 * 1024 * 1024,
            max_mask_bytes: 32 * 1024 * 1024,
            max_storage_bytes: 64 * 1024 * 1024,
            max_commands_per_path: 1024,
            max_total_commands: 65_536,
            max_events: 4096,
            max_clip_depth: 128,
            max_work: 100_000_000,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RasterErrorKind {
    Unsupported(&'static str),
    Limit(&'static str),
    Allocation,
    Geometry,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RasterError {
    pub span: Range<usize>,
    pub kind: RasterErrorKind,
}
impl fmt::Display for RasterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "raster error at {}..{}: {:?}",
            self.span.start, self.span.end, self.kind
        )
    }
}
impl std::error::Error for RasterError {}
fn err(span: &Range<usize>, kind: RasterErrorKind) -> RasterError {
    RasterError {
        span: span.clone(),
        kind,
    }
}
fn unsupported(span: &Range<usize>, why: &'static str) -> RasterError {
    err(span, RasterErrorKind::Unsupported(why))
}
fn limit(span: &Range<usize>, why: &'static str) -> RasterError {
    err(span, RasterErrorKind::Limit(why))
}
fn allocate<T>(count: usize, span: &Range<usize>) -> Result<Vec<T>, RasterError> {
    let mut v = Vec::new();
    v.try_reserve_exact(count)
        .map_err(|_| err(span, RasterErrorKind::Allocation))?;
    Ok(v)
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RasterUsage {
    pub planned_storage_bytes: usize,
    pub mask_bytes: usize,
    pub transformed_commands: usize,
    pub work: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PixelPoint {
    x: i32,
    y: i32,
}
#[derive(Debug)]
struct Polygon {
    points: Vec<PixelPoint>,
    ends: Vec<usize>,
}
#[derive(Debug)]
struct PlannedClip {
    polygon: Polygon,
    parent: Option<usize>,
    rule: FillRule,
    depth: usize,
    span: Range<usize>,
}
#[derive(Debug)]
struct PlannedFill {
    polygon: Polygon,
    clip: Option<usize>,
    rule: FillRule,
    color: [u8; 3],
    span: Range<usize>,
}
#[derive(Debug)]
enum BaseClip {
    Unbounded,
    Empty,
    Polygon(Polygon),
}
#[derive(Debug)]
pub struct RasterPlan {
    target: RasterTarget,
    base: BaseClip,
    clips: Vec<PlannedClip>,
    fills: Vec<PlannedFill>,
    usage: RasterUsage,
}
#[derive(Debug, PartialEq)]
pub struct RasterImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
impl RasterImage {
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
struct Budget {
    limits: RasterLimits,
    usage: RasterUsage,
}
impl Budget {
    fn storage(&mut self, n: usize, s: &Range<usize>) -> Result<(), RasterError> {
        self.usage.planned_storage_bytes = self
            .usage
            .planned_storage_bytes
            .checked_add(n)
            .ok_or_else(|| limit(s, "storage"))?;
        if self.usage.planned_storage_bytes > self.limits.max_storage_bytes {
            return Err(limit(s, "storage"));
        }
        Ok(())
    }
    fn work(&mut self, n: u64, s: &Range<usize>) -> Result<(), RasterError> {
        self.usage.work = self
            .usage
            .work
            .checked_add(n)
            .ok_or_else(|| limit(s, "work"))?;
        if self.usage.work > self.limits.max_work {
            return Err(limit(s, "work"));
        }
        Ok(())
    }
    fn commands(&mut self, n: usize, s: &Range<usize>) -> Result<(), RasterError> {
        if n > self.limits.max_commands_per_path.min(1024) {
            return Err(limit(s, "commands per path"));
        }
        self.usage.transformed_commands = self
            .usage
            .transformed_commands
            .checked_add(n)
            .ok_or_else(|| limit(s, "total commands"))?;
        if self.usage.transformed_commands > self.limits.max_total_commands {
            return Err(limit(s, "total commands"));
        }
        self.storage(
            n.checked_mul(size_of::<PixelPoint>() + size_of::<usize>())
                .ok_or_else(|| limit(s, "storage"))?,
            s,
        )?;
        self.work(n as u64, s)
    }
    fn scan(&mut self, p: &Polygon, t: RasterTarget, s: &Range<usize>) -> Result<(), RasterError> {
        if p.points.is_empty() {
            return Ok(());
        }
        let e = p.points.len() as u64 + 2;
        self.work(
            (u64::from(t.height) + 2) * e * e + u64::from(t.width) * u64::from(t.height),
            s,
        )
    }
}
fn pixel<S>(
    p: Point<S>,
    map: Affine<S, DeviceSpace>,
    t: RasterTarget,
    s: &Range<usize>,
) -> Result<PixelPoint, RasterError> {
    let p = map
        .transform_point(p)
        .map_err(|_| err(s, RasterErrorKind::Geometry))?;
    if p.x().fract() != 0. || p.y().fract() != 0. {
        return Err(unsupported(s, "fractional device coordinate"));
    }
    if p.x() < 0. || p.y() < 0. || p.x() > f64::from(t.width) || p.y() > f64::from(t.height) {
        return Err(unsupported(s, "vertex outside canvas"));
    }
    Ok(PixelPoint {
        x: p.x() as i32,
        y: p.y() as i32,
    })
}
fn intersects(a: PixelPoint, b: PixelPoint, c: PixelPoint, d: PixelPoint) -> bool {
    a.x.min(b.x) <= c.x.max(d.x)
        && c.x.min(d.x) <= a.x.max(b.x)
        && a.y.min(b.y) <= c.y.max(d.y)
        && c.y.min(d.y) <= a.y.max(b.y)
}
fn finish_contour(
    p: &mut Polygon,
    start: usize,
    had_line: bool,
    b: &mut Budget,
    s: &Range<usize>,
) -> Result<(), RasterError> {
    if !had_line {
        p.points.truncate(start);
        return Ok(());
    }
    if p.points.len() > start + 1 && p.points.last() == p.points.get(start) {
        p.points.pop();
    }
    let pts = &p.points[start..];
    let n = pts.len();
    if n < 4 {
        return Err(unsupported(s, "degenerate contour"));
    }
    b.work((n as u64) * (n as u64), s)?;
    let mut area = 0_i64;
    for i in 0..n {
        let a = pts[i];
        let z = pts[(i + 1) % n];
        if a == z {
            return Err(unsupported(s, "degenerate edge"));
        }
        if a.x != z.x && a.y != z.y {
            return Err(unsupported(s, "slanted edge"));
        }
        area += i64::from(a.x) * i64::from(z.y) - i64::from(z.x) * i64::from(a.y);
        for j in i + 1..n {
            let c = pts[j];
            let d = pts[(j + 1) % n];
            if j == i + 1 || i == 0 && j == n - 1 {
                // Adjacent collinear reversals overlap beyond their shared endpoint.
                if a.x == z.x && c.x == d.x && a.x == c.x {
                    let overlap = a.y.max(z.y).min(c.y.max(d.y)) - a.y.min(z.y).max(c.y.min(d.y));
                    if overlap > 0 {
                        return Err(unsupported(s, "self-intersecting contour"));
                    }
                }
                if a.y == z.y && c.y == d.y && a.y == c.y {
                    let overlap = a.x.max(z.x).min(c.x.max(d.x)) - a.x.min(z.x).max(c.x.min(d.x));
                    if overlap > 0 {
                        return Err(unsupported(s, "self-intersecting contour"));
                    }
                }
            } else if intersects(a, z, c, d) {
                return Err(unsupported(s, "self-intersecting contour"));
            }
        }
    }
    if area == 0 {
        return Err(unsupported(s, "zero-area contour"));
    }
    p.ends.push(p.points.len());
    Ok(())
}
fn polygon(
    commands: &[PathCommand],
    map: Affine<UserSpace, DeviceSpace>,
    t: RasterTarget,
    b: &mut Budget,
    s: &Range<usize>,
) -> Result<Polygon, RasterError> {
    b.commands(commands.len(), s)?;
    let mut p = Polygon {
        points: allocate(commands.len(), s)?,
        ends: allocate(commands.len(), s)?,
    };
    let mut start = 0;
    let mut had = false;
    let mut current = None;
    let mut origin = None;
    for command in commands {
        match *command {
            PathCommand::MoveTo(v) => {
                finish_contour(&mut p, start, had, b, s)?;
                start = p.points.len();
                let v = pixel(v, map, t, s)?;
                p.points.push(v);
                current = Some(v);
                origin = Some(v);
                had = false;
            }
            PathCommand::LineTo(v) => {
                let v = pixel(v, map, t, s)?;
                if p.points.len() == start {
                    p.points
                        .push(current.ok_or_else(|| unsupported(s, "line without current point"))?);
                }
                if p.points.last() != Some(&v) {
                    p.points.push(v);
                }
                current = Some(v);
                had = true;
            }
            PathCommand::ClosePath => {
                finish_contour(&mut p, start, had, b, s)?;
                start = p.points.len();
                current = origin;
                had = false;
            }
            PathCommand::CubicTo { .. } => return Err(unsupported(s, "cubic path")),
        }
    }
    finish_contour(&mut p, start, had, b, s)?;
    Ok(p)
}
fn color(c: DeviceColor, s: &Range<usize>) -> Result<[u8; 3], RasterError> {
    let v = match c {
        DeviceColor::Gray(g) => [g; 3],
        DeviceColor::Rgb(rgb) => rgb,
        DeviceColor::Cmyk(_) => return Err(unsupported(s, "CMYK paint")),
    };
    Ok(v.map(|x| (x * 255. + 0.5).floor() as u8))
}

pub fn prepare_orthogonal_raster(
    program: &PathProgram,
    target: RasterTarget,
    limits: RasterLimits,
) -> Result<RasterPlan, RasterError> {
    let s = 0..0;
    if target.width == 0 || target.height == 0 || target.width > 4096 || target.height > 4096 {
        return Err(limit(&s, "canvas dimensions"));
    }
    let pixels = target.width as usize * target.height as usize;
    if pixels > limits.max_pixels.min(4_194_304) {
        return Err(limit(&s, "pixels"));
    }
    let output = pixels * 4;
    if output > limits.max_output_bytes {
        return Err(limit(&s, "output bytes"));
    }
    if program.events().len() > limits.max_events.min(4096) {
        return Err(limit(&s, "events"));
    }
    let mut b = Budget {
        limits,
        usage: RasterUsage::default(),
    };
    b.storage(output, &s)?;
    b.work(pixels as u64, &s)?;
    // Conservative allowance for one bounded backend edge vector/sort scratch and
    // one path. Backend allocates sequentially; process OOM remains nonrecoverable.
    b.storage(1024 * 1024, &s)?;
    b.storage(
        program
            .clips()
            .len()
            .checked_mul(size_of::<PlannedClip>() + size_of::<tiny_skia::Mask>())
            .ok_or_else(|| limit(&s, "storage"))?,
        &s,
    )?;
    b.storage(
        program
            .events()
            .len()
            .checked_mul(size_of::<PlannedFill>())
            .ok_or_else(|| limit(&s, "storage"))?,
        &s,
    )?;
    let base = match program.initial_clip() {
        InitialClip::Unbounded => BaseClip::Unbounded,
        InitialClip::Empty => BaseClip::Empty,
        InitialClip::Rectangle(r) => {
            if r.is_empty() {
                BaseClip::Empty
            } else {
                b.commands(5, &s)?;
                let [x0, y0, x1, y1] = r.bounds();
                let mut p = Polygon {
                    points: allocate(5, &s)?,
                    ends: allocate(5, &s)?,
                };
                for [x, y] in [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] {
                    p.points.push(pixel(
                        Point::<PageSpace>::new(x, y)
                            .map_err(|_| err(&s, RasterErrorKind::Geometry))?,
                        target.page_to_device,
                        target,
                        &s,
                    )?);
                }
                finish_contour(&mut p, 0, true, &mut b, &s)?;
                b.scan(&p, target, &s)?;
                BaseClip::Polygon(p)
            }
        }
    };
    let masks = program.clips().len() + usize::from(!matches!(base, BaseClip::Unbounded));
    let mask_bytes = masks
        .checked_mul(pixels)
        .ok_or_else(|| limit(&s, "mask bytes"))?;
    if mask_bytes > limits.max_mask_bytes {
        return Err(limit(&s, "mask bytes"));
    }
    b.usage.mask_bytes = mask_bytes;
    b.storage(mask_bytes, &s)?;
    b.work(mask_bytes as u64, &s)?;
    let mut clips: Vec<PlannedClip> = allocate(program.clips().len(), &s)?;
    let mut fills = allocate(program.events().len(), &s)?;
    for event in program.events() {
        match event {
            PathEvent::Stroke { paint, .. } => return Err(unsupported(&paint.span(), "stroke")),
            PathEvent::Fill { paint, rule } => {
                let span = paint.span();
                let map = paint
                    .ctm()
                    .then(target.page_to_device)
                    .map_err(|_| err(&span, RasterErrorKind::Geometry))?;
                let poly = polygon(
                    program.paths()[paint.path().index()].commands(),
                    map,
                    target,
                    &mut b,
                    &span,
                )?;
                b.scan(&poly, target, &span)?;
                fills.push(PlannedFill {
                    polygon: poly,
                    clip: paint.clip().map(|v| v.index()),
                    rule: *rule,
                    color: color(paint.color(), &span)?,
                    span,
                });
            }
            PathEvent::ClipPush { id, span } => {
                if id.index() != clips.len() {
                    return Err(unsupported(span, "nonsequential clip arena"));
                }
                let node = &program.clips()[id.index()];
                let parent = node.parent().map(|v| v.index());
                let depth = match parent {
                    Some(i) => {
                        clips
                            .get(i)
                            .ok_or_else(|| unsupported(span, "invalid clip parent"))?
                            .depth
                            + 1
                    }
                    None => 1,
                };
                if depth > limits.max_clip_depth {
                    return Err(limit(span, "clip depth"));
                }
                let map = node
                    .ctm()
                    .then(target.page_to_device)
                    .map_err(|_| err(span, RasterErrorKind::Geometry))?;
                let poly = polygon(
                    program.paths()[node.path().index()].commands(),
                    map,
                    target,
                    &mut b,
                    span,
                )?;
                b.scan(&poly, target, span)?;
                if parent.is_some() || !matches!(base, BaseClip::Unbounded) {
                    b.work(pixels as u64, span)?;
                }
                clips.push(PlannedClip {
                    polygon: poly,
                    parent,
                    rule: node.rule(),
                    depth,
                    span: span.clone(),
                });
            }
            PathEvent::ClipRestore { .. } => {}
        }
    }
    Ok(RasterPlan {
        target,
        base,
        clips,
        fills,
        usage: b.usage,
    })
}
fn backend_path(p: &Polygon, s: &Range<usize>) -> Result<Option<tiny_skia::Path>, RasterError> {
    if p.points.is_empty() {
        return Ok(None);
    }
    let mut builder =
        tiny_skia::PathBuilder::with_capacity(p.points.len() + p.ends.len(), p.points.len());
    let mut start = 0;
    for &end in &p.ends {
        let first = p.points[start];
        builder.move_to(first.x as f32, first.y as f32);
        for v in &p.points[start + 1..end] {
            builder.line_to(v.x as f32, v.y as f32);
        }
        builder.close();
        start = end;
    }
    builder
        .finish()
        .map(Some)
        .ok_or_else(|| err(s, RasterErrorKind::Geometry))
}
fn empty_mask(t: RasterTarget, s: &Range<usize>) -> Result<tiny_skia::Mask, RasterError> {
    let n = t.width as usize * t.height as usize;
    let mut bytes = allocate(n, s)?;
    bytes.resize(n, 0);
    let size = tiny_skia::IntSize::from_wh(t.width, t.height)
        .ok_or_else(|| err(s, RasterErrorKind::Geometry))?;
    tiny_skia::Mask::from_vec(bytes, size).ok_or_else(|| err(s, RasterErrorKind::Geometry))
}
fn backend_rule(r: FillRule) -> tiny_skia::FillRule {
    match r {
        FillRule::NonZero => tiny_skia::FillRule::Winding,
        FillRule::EvenOdd => tiny_skia::FillRule::EvenOdd,
    }
}
fn fill_mask(
    mask: &mut tiny_skia::Mask,
    p: &Polygon,
    rule: FillRule,
    s: &Range<usize>,
) -> Result<(), RasterError> {
    if let Some(path) = backend_path(p, s)? {
        mask.fill_path(
            &path,
            backend_rule(rule),
            false,
            tiny_skia::Transform::identity(),
        );
    }
    Ok(())
}
impl RasterPlan {
    pub fn usage(&self) -> RasterUsage {
        self.usage
    }
    pub fn render(&self) -> Result<RasterImage, RasterError> {
        let span = 0..0;
        let base = match &self.base {
            BaseClip::Unbounded => None,
            BaseClip::Empty => Some(empty_mask(self.target, &span)?),
            BaseClip::Polygon(p) => {
                let mut mask = empty_mask(self.target, &span)?;
                fill_mask(&mut mask, p, FillRule::NonZero, &span)?;
                Some(mask)
            }
        };
        let mut masks: Vec<tiny_skia::Mask> = allocate(self.clips.len(), &span)?;
        for clip in &self.clips {
            let mut mask = empty_mask(self.target, &clip.span)?;
            fill_mask(&mut mask, &clip.polygon, clip.rule, &clip.span)?;
            let parent = clip.parent.map(|i| &masks[i]).or(base.as_ref());
            if let Some(parent) = parent {
                for (out, &p) in mask.data_mut().iter_mut().zip(parent.data()) {
                    *out &= p;
                }
            }
            masks.push(mask);
        }
        let len = self.target.width as usize * self.target.height as usize * 4;
        let mut bytes = allocate(len, &span)?;
        bytes.resize(len, 255);
        let size = tiny_skia::IntSize::from_wh(self.target.width, self.target.height)
            .ok_or_else(|| err(&span, RasterErrorKind::Geometry))?;
        let mut pix = tiny_skia::Pixmap::from_vec(bytes, size)
            .ok_or_else(|| err(&span, RasterErrorKind::Geometry))?;
        for fill in &self.fills {
            if let Some(path) = backend_path(&fill.polygon, &fill.span)? {
                let [r, g, b] = fill.color;
                let mut paint = tiny_skia::Paint::default();
                paint.set_color_rgba8(r, g, b, 255);
                paint.anti_alias = false;
                paint.colorspace = tiny_skia::ColorSpace::Linear;
                paint.blend_mode = tiny_skia::BlendMode::SourceOver;
                let mask = fill.clip.map(|i| &masks[i]).or(base.as_ref());
                pix.fill_path(
                    &path,
                    &paint,
                    backend_rule(fill.rule),
                    tiny_skia::Transform::identity(),
                    mask,
                );
            }
        }
        Ok(RasterImage {
            width: self.target.width,
            height: self.target.height,
            pixels: pix.take(),
        })
    }
}
pub fn render_orthogonal_raster(
    program: &PathProgram,
    target: RasterTarget,
    limits: RasterLimits,
) -> Result<RasterImage, RasterError> {
    prepare_orthogonal_raster(program, target, limits)?.render()
}
