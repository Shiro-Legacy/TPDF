// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded stroke expansion into owned, consistently oriented components.
//! Dashes are cut from the user-space centerline first; each "on" piece is then
//! expanded as an open subpath with the configured caps and joins.
//! User-space widths are expanded before the caller's affine raster transform.
//! Output counters charge emitted component points/commands before arena storage;
//! temporary candidate vertices and transforms instead consume precharged work.
//! Each DFS uses a fixed 25-slot stack (no heap); vector full-request reservations
//! have a separate ledger. All failed work/reservations remain cumulative.
use crate::geometry::{Affine, DeviceSpace, GeometryError, Point, UserSpace};
use crate::path::{DashPattern, LineCap, LineJoin, PathCommand, StrokeStyle};
use std::{fmt, mem::size_of};
const USER_MAX: f64 = (1u64 << 40) as f64;
const DEVICE_MAX: f64 = (1u32 << 21) as f64;
const EPS: f64 = 1. / 256.;
const DEPTH: usize = 24;
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StrokeErrorKind {
    Unsupported(&'static str),
    Limit(&'static str),
    Arithmetic(&'static str),
    Geometry(GeometryError),
    Allocation,
    Invariant(&'static str),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrokeError {
    pub command_index: Option<usize>,
    pub kind: StrokeErrorKind,
}
impl fmt::Display for StrokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "stroke command {:?}: {:?}",
            self.command_index, self.kind
        )
    }
}
impl std::error::Error for StrokeError {}
fn err(kind: StrokeErrorKind) -> StrokeError {
    StrokeError {
        command_index: None,
        kind,
    }
}
#[derive(Clone, Copy, Debug)]
pub struct StrokeLimits {
    pub max_attempts: usize,
    pub max_input_commands: usize,
    pub max_subpaths: usize,
    pub max_line_segments: usize,
    pub max_curve_nodes: usize,
    pub max_curve_leaves: usize,
    pub max_join_attempts: usize,
    pub max_cap_attempts: usize,
    pub max_arc_nodes: usize,
    pub max_arc_vertices: usize,
    pub max_components: usize,
    pub max_output_commands: usize,
    pub max_output_points: usize,
    pub max_allocated_bytes: usize,
    pub max_work: u64,
    pub max_subdivision_depth: usize,
    pub max_arc_depth: usize,
}
impl Default for StrokeLimits {
    fn default() -> Self {
        Self {
            max_attempts: 10_000,
            max_input_commands: 100_000,
            max_subpaths: 10_000,
            max_line_segments: 100_000,
            max_curve_nodes: 1_000_000,
            max_curve_leaves: 500_000,
            max_join_attempts: 200_000,
            max_cap_attempts: 200_000,
            max_arc_nodes: 1_000_000,
            max_arc_vertices: 500_000,
            max_components: 1_000_000,
            max_output_commands: 5_000_000,
            max_output_points: 4_000_000,
            max_allocated_bytes: 256 << 20,
            max_work: 100_000_000,
            max_subdivision_depth: 24,
            max_arc_depth: 24,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StrokeUsage {
    pub attempts: usize,
    pub input_commands: usize,
    pub subpaths: usize,
    pub line_segments: usize,
    pub curve_nodes: usize,
    pub curve_leaves: usize,
    pub join_attempts: usize,
    pub cap_attempts: usize,
    pub arc_nodes: usize,
    pub arc_vertices: usize,
    pub components: usize,
    pub output_commands: usize,
    pub output_points: usize,
    pub allocated_bytes: usize,
    pub work: u64,
}
macro_rules! counters {
    ($f:ident) => {
        $f!(max_attempts, attempts);
        $f!(max_input_commands, input_commands);
        $f!(max_subpaths, subpaths);
        $f!(max_line_segments, line_segments);
        $f!(max_curve_nodes, curve_nodes);
        $f!(max_curve_leaves, curve_leaves);
        $f!(max_join_attempts, join_attempts);
        $f!(max_cap_attempts, cap_attempts);
        $f!(max_arc_nodes, arc_nodes);
        $f!(max_arc_vertices, arc_vertices);
        $f!(max_components, components);
        $f!(max_output_commands, output_commands);
        $f!(max_output_points, output_points);
        $f!(max_allocated_bytes, allocated_bytes);
        $f!(max_work, work);
    };
}
impl StrokeLimits {
    pub fn intersect(mut self, other: Self) -> Self {
        macro_rules! f {
            ($m:ident,$u:ident) => {
                self.$m = self.$m.min(other.$m);
            };
        }
        counters!(f);
        self.max_subdivision_depth = self
            .max_subdivision_depth
            .min(other.max_subdivision_depth)
            .min(DEPTH);
        self.max_arc_depth = self.max_arc_depth.min(other.max_arc_depth).min(DEPTH);
        self
    }
    pub fn remaining(mut self, used: StrokeUsage) -> Result<Self, StrokeError> {
        macro_rules! f {
            ($m:ident,$u:ident) => {
                self.$m = self
                    .$m
                    .checked_sub(used.$u)
                    .ok_or_else(|| err(StrokeErrorKind::Arithmetic("inconsistent usage")))?;
            };
        }
        counters!(f);
        Ok(self)
    }
}
impl StrokeUsage {
    pub fn accumulate(&mut self, other: Self) -> Result<(), StrokeError> {
        let mut sum = *self;
        macro_rules! f {
            ($m:ident,$u:ident) => {
                sum.$u = sum
                    .$u
                    .checked_add(other.$u)
                    .ok_or_else(|| err(StrokeErrorKind::Arithmetic("usage overflow")))?;
            };
        }
        counters!(f);
        *self = sum;
        Ok(())
    }
}
pub fn validate_style(style: StrokeStyle, dash: Option<&DashPattern>) -> Result<(), StrokeError> {
    if !style.width().is_finite()
        || !style.miter_limit().is_finite()
        || !style.dash_phase().is_finite()
        || style.width() < 0.
        || style.miter_limit() < 1.
        || style.dash_phase() < 0.
    {
        return Err(err(StrokeErrorKind::Arithmetic("invalid stroke style")));
    }
    if style.width() == 0. {
        return Err(err(StrokeErrorKind::Unsupported("hairline stroke")));
    }
    if style.width() * 0.5 > USER_MAX {
        return Err(err(StrokeErrorKind::Limit("stroke radius")));
    }
    if style.dash().is_some() != dash.is_some() {
        return Err(err(StrokeErrorKind::Invariant("dash pattern binding")));
    }
    if let Some(values) = dash.map(DashPattern::values) {
        if values.iter().any(|v| !v.is_finite() || *v < 0.) {
            return Err(err(StrokeErrorKind::Arithmetic("invalid dash array")));
        }
        if values.iter().sum::<f64>() > USER_MAX {
            return Err(err(StrokeErrorKind::Limit("dash period")));
        }
        // Zero-length "on" entries draw cap-only dots, which the expander
        // cannot represent yet. With butt caps they paint nothing.
        if style.cap() != LineCap::Butt && values.iter().step_by(2).any(|v| *v == 0.) {
            return Err(err(StrokeErrorKind::Unsupported(
                "zero-length dash with caps",
            )));
        }
    }
    Ok(())
}
#[derive(Debug)]
pub struct StrokeOutline {
    commands: Vec<PathCommand>,
}
impl StrokeOutline {
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }
}
pub struct StrokeSession {
    limits: StrokeLimits,
    usage: StrokeUsage,
}
impl StrokeSession {
    pub fn new(limits: StrokeLimits) -> Self {
        Self {
            limits,
            usage: StrokeUsage::default(),
        }
    }
    pub fn usage(&self) -> StrokeUsage {
        self.usage
    }
    pub fn expand(
        &mut self,
        commands: &[PathCommand],
        style: StrokeStyle,
        dash: Option<&DashPattern>,
        map: Affine<UserSpace, DeviceSpace>,
        limits: StrokeLimits,
    ) -> Result<StrokeOutline, StrokeError> {
        let limits = limits.intersect(self.limits.remaining(self.usage)?);
        let mut b = Budget {
            limits,
            usage: StrokeUsage::default(),
            index: None,
        };
        let result = (|| {
            b.count(Counter::Attempts, 1)?;
            b.work(8)?;
            validate_style(style, dash)?;
            match dash.map(DashPattern::values) {
                Some(values) if values.iter().sum::<f64>() > 0. => {
                    let dashed = dash_centerline(commands, values, style.dash_phase(), &mut b)?;
                    expand(&dashed, style, map, &mut b)
                }
                _ => expand(commands, style, map, &mut b),
            }
        })();
        self.usage.accumulate(b.usage)?;
        result
    }
}
struct Budget {
    limits: StrokeLimits,
    usage: StrokeUsage,
    index: Option<usize>,
}
#[derive(Clone, Copy)]
enum Counter {
    Attempts,
    Input,
    Subpaths,
    Lines,
    CurveNodes,
    CurveLeaves,
    Joins,
    Caps,
    ArcNodes,
    ArcVertices,
    Components,
    Commands,
    Points,
    Bytes,
}
impl Budget {
    fn error(&self, kind: StrokeErrorKind) -> StrokeError {
        StrokeError {
            command_index: self.index,
            kind,
        }
    }
    fn count(&mut self, c: Counter, n: usize) -> Result<(), StrokeError> {
        let (v, cap, name) = match c {
            Counter::Attempts => (
                &mut self.usage.attempts,
                self.limits.max_attempts,
                "attempts",
            ),
            Counter::Input => (
                &mut self.usage.input_commands,
                self.limits.max_input_commands,
                "input commands",
            ),
            Counter::Subpaths => (
                &mut self.usage.subpaths,
                self.limits.max_subpaths,
                "subpaths",
            ),
            Counter::Lines => (
                &mut self.usage.line_segments,
                self.limits.max_line_segments,
                "line segments",
            ),
            Counter::CurveNodes => (
                &mut self.usage.curve_nodes,
                self.limits.max_curve_nodes,
                "curve nodes",
            ),
            Counter::CurveLeaves => (
                &mut self.usage.curve_leaves,
                self.limits.max_curve_leaves,
                "curve leaves",
            ),
            Counter::Joins => (
                &mut self.usage.join_attempts,
                self.limits.max_join_attempts,
                "join attempts",
            ),
            Counter::Caps => (
                &mut self.usage.cap_attempts,
                self.limits.max_cap_attempts,
                "cap attempts",
            ),
            Counter::ArcNodes => (
                &mut self.usage.arc_nodes,
                self.limits.max_arc_nodes,
                "arc nodes",
            ),
            Counter::ArcVertices => (
                &mut self.usage.arc_vertices,
                self.limits.max_arc_vertices,
                "arc vertices",
            ),
            Counter::Components => (
                &mut self.usage.components,
                self.limits.max_components,
                "components",
            ),
            Counter::Commands => (
                &mut self.usage.output_commands,
                self.limits.max_output_commands,
                "output commands",
            ),
            Counter::Points => (
                &mut self.usage.output_points,
                self.limits.max_output_points,
                "output points",
            ),
            Counter::Bytes => (
                &mut self.usage.allocated_bytes,
                self.limits.max_allocated_bytes,
                "allocated bytes",
            ),
        };
        *v = v.checked_add(n).filter(|n| *n <= cap).ok_or(StrokeError {
            command_index: self.index,
            kind: StrokeErrorKind::Limit(name),
        })?;
        Ok(())
    }
    fn work(&mut self, n: u64) -> Result<(), StrokeError> {
        self.usage.work = self
            .usage
            .work
            .checked_add(n)
            .filter(|v| *v <= self.limits.max_work)
            .ok_or_else(|| self.error(StrokeErrorKind::Limit("work")))?;
        Ok(())
    }
}
/// Samples per cubic for arc-length lookup. Dash boundaries on curves are
/// placed by chord length, within a small fraction of the dash length.
const DASH_CURVE_SAMPLES: usize = 32;
struct Dasher<'a> {
    pattern: &'a [f64],
    phase: f64,
    index: usize,
    left: f64,
    on: bool,
    out: Vec<PathCommand>,
    pen_down: bool,
}
impl Dasher<'_> {
    fn restart(&mut self) {
        let period: f64 = self.pattern.iter().sum();
        let mut phase = self.phase % period;
        self.index = 0;
        while phase >= self.pattern[self.index] {
            phase -= self.pattern[self.index];
            self.index = (self.index + 1) % self.pattern.len();
        }
        self.left = self.pattern[self.index] - phase;
        self.on = self.index % 2 == 0;
        self.pen_down = false;
    }
    fn advance(&mut self) {
        self.index = (self.index + 1) % self.pattern.len();
        self.left = self.pattern[self.index];
        self.on = self.index % 2 == 0;
        self.pen_down = false;
    }
    /// Walk one segment of `length`; `piece(t0, t1)` emits the drawing command
    /// for the parameter range and `at(t)` gives the point for a new dash.
    fn segment(
        &mut self,
        length: f64,
        to_t: &dyn Fn(f64) -> f64,
        at: &dyn Fn(f64) -> Point<UserSpace>,
        piece: &dyn Fn(f64, f64) -> PathCommand,
        b: &mut Budget,
    ) -> Result<(), StrokeError> {
        let mut done = 0.;
        while done < length {
            b.work(4)?;
            let step = self.left.min(length - done);
            if self.on && step > 0. {
                let (t0, t1) = (to_t(done), to_t(done + step));
                b.count(Counter::Input, 2)?;
                if !self.pen_down {
                    self.out.push(PathCommand::MoveTo(at(t0)));
                    self.pen_down = true;
                }
                self.out.push(piece(t0, t1));
            }
            done += step;
            self.left -= step;
            if self.left <= 0. {
                self.advance();
            }
        }
        Ok(())
    }
}
fn lerp(a: Point<UserSpace>, z: Point<UserSpace>, t: f64) -> Point<UserSpace> {
    let ([ax, ay], [zx, zy]) = (a.coordinates(), z.coordinates());
    Point::new(ax + (zx - ax) * t, ay + (zy - ay) * t).expect("finite interpolation")
}
fn cubic_at(p: [Point<UserSpace>; 4], t: f64) -> Point<UserSpace> {
    let ab = lerp(p[0], p[1], t);
    let bc = lerp(p[1], p[2], t);
    let cd = lerp(p[2], p[3], t);
    lerp(lerp(ab, bc, t), lerp(bc, cd, t), t)
}
/// Exact de Casteljau restriction of a cubic to [t0, t1].
fn cubic_piece(p: [Point<UserSpace>; 4], t0: f64, t1: f64) -> PathCommand {
    let split = |p: [Point<UserSpace>; 4], t: f64| {
        let ab = lerp(p[0], p[1], t);
        let bc = lerp(p[1], p[2], t);
        let cd = lerp(p[2], p[3], t);
        let (abc, bcd) = (lerp(ab, bc, t), lerp(bc, cd, t));
        let mid = lerp(abc, bcd, t);
        ([p[0], ab, abc, mid], [mid, bcd, cd, p[3]])
    };
    let tail = if t0 > 0. { split(p, t0).1 } else { p };
    let q = if t1 < 1. {
        split(tail, (t1 - t0) / (1. - t0)).0
    } else {
        tail
    };
    PathCommand::CubicTo {
        control1: q[1],
        control2: q[2],
        end: q[3],
    }
}
fn distance(a: Point<UserSpace>, z: Point<UserSpace>) -> f64 {
    let ([ax, ay], [zx, zy]) = (a.coordinates(), z.coordinates());
    (zx - ax).hypot(zy - ay)
}
/// Cut a centerline into "on" dash pieces. The pattern restarts at the phase
/// for every subpath; ClosePath dashes its closing line, and the dashes at a
/// closed subpath's seam are capped separately.
// ponytail: seam dashes are not joined across the close; join them if a
// reference comparison shows the visible cap difference matters.
fn dash_centerline(
    commands: &[PathCommand],
    values: &[f64],
    phase: f64,
    b: &mut Budget,
) -> Result<Vec<PathCommand>, StrokeError> {
    let doubled;
    let pattern = if values.len() % 2 == 1 {
        doubled = [values, values].concat();
        &doubled[..]
    } else {
        values
    };
    let mut d = Dasher {
        pattern,
        phase,
        index: 0,
        left: 0.,
        on: false,
        out: Vec::new(),
        pen_down: false,
    };
    let mut current: Option<Point<UserSpace>> = None;
    let mut start = None;
    for (index, command) in commands.iter().enumerate() {
        b.index = Some(index);
        b.work(8)?;
        match *command {
            PathCommand::MoveTo(p) => {
                d.restart();
                current = Some(p);
                start = Some(p);
            }
            PathCommand::LineTo(_) | PathCommand::ClosePath if current.is_some() => {
                let a = current.expect("checked");
                let z = match *command {
                    PathCommand::LineTo(z) => z,
                    _ => start.expect("subpath start"),
                };
                let length = distance(a, z);
                d.segment(
                    length,
                    &|s| s / length,
                    &|t| lerp(a, z, t),
                    &|_, t1| PathCommand::LineTo(lerp(a, z, t1)),
                    b,
                )?;
                current = Some(z);
                if matches!(command, PathCommand::ClosePath) {
                    d.restart();
                }
            }
            PathCommand::CubicTo {
                control1,
                control2,
                end,
            } if current.is_some() => {
                let p = [current.expect("checked"), control1, control2, end];
                b.work(DASH_CURVE_SAMPLES as u64 * 8)?;
                let mut lengths = [0.; DASH_CURVE_SAMPLES + 1];
                let mut previous = p[0];
                for i in 1..=DASH_CURVE_SAMPLES {
                    let next = cubic_at(p, i as f64 / DASH_CURVE_SAMPLES as f64);
                    lengths[i] = lengths[i - 1] + distance(previous, next);
                    previous = next;
                }
                let total = lengths[DASH_CURVE_SAMPLES];
                let to_t = |s: f64| {
                    let i = lengths
                        .partition_point(|&l| l < s)
                        .clamp(1, DASH_CURVE_SAMPLES);
                    let span = lengths[i] - lengths[i - 1];
                    let within = if span > 0. {
                        (s - lengths[i - 1]) / span
                    } else {
                        0.
                    };
                    ((i - 1) as f64 + within.clamp(0., 1.)) / DASH_CURVE_SAMPLES as f64
                };
                d.segment(
                    total,
                    &to_t,
                    &|t| cubic_at(p, t),
                    &|t0, t1| cubic_piece(p, t0, t1),
                    b,
                )?;
                current = Some(end);
            }
            _ => {
                return Err(b.error(StrokeErrorKind::Invariant("dash segment without move")));
            }
        }
    }
    b.index = None;
    Ok(d.out)
}
fn expand(
    commands: &[PathCommand],
    style: StrokeStyle,
    map: Affine<UserSpace, DeviceSpace>,
    b: &mut Budget,
) -> Result<StrokeOutline, StrokeError> {
    build(commands, style, map, b)
}

// Every finite f64 input is an exact dyadic singleton. Operations enclose real
// results; an invalid enclosure propagates until a predicate refuses it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct I {
    lo: f64,
    hi: f64,
}
fn up(v: f64) -> f64 {
    if v == 0. {
        f64::from_bits(1)
    } else if v > 0. {
        f64::from_bits(v.to_bits() + 1)
    } else {
        f64::from_bits(v.to_bits() - 1)
    }
}
fn down(v: f64) -> f64 {
    -up(-v)
}
impl I {
    const BAD: Self = Self {
        lo: f64::NAN,
        hi: f64::NAN,
    };
    fn p(v: f64) -> Self {
        if v.is_finite() {
            Self { lo: v, hi: v }
        } else {
            Self::BAD
        }
    }
    fn bounds(lo: f64, hi: f64) -> Self {
        if lo.is_finite() && hi.is_finite() && lo <= hi {
            Self { lo, hi }
        } else {
            Self::BAD
        }
    }
    fn out(lo: f64, hi: f64) -> Self {
        if !lo.is_finite() || !hi.is_finite() {
            return Self::BAD;
        }
        Self::bounds(down(lo), up(hi))
    }
    fn ok(self) -> bool {
        self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi
    }
    fn zero(self) -> bool {
        self.lo == 0. && self.hi == 0.
    }
    fn add(self, b: Self) -> Self {
        if !self.ok() || !b.ok() {
            return Self::BAD;
        }
        if self.zero() {
            return b;
        }
        if b.zero() {
            return self;
        }
        Self::out(self.lo + b.lo, self.hi + b.hi)
    }
    fn neg(self) -> Self {
        Self {
            lo: -self.hi,
            hi: -self.lo,
        }
    }
    fn sub(self, b: Self) -> Self {
        if self.ok() && self.lo == self.hi && self == b {
            return Self::p(0.);
        }
        self.add(b.neg())
    }
    fn mul(self, b: Self) -> Self {
        if !self.ok() || !b.ok() {
            return Self::BAD;
        }
        if self.zero() || b.zero() {
            return Self::p(0.);
        }
        let p = [
            self.lo * b.lo,
            self.lo * b.hi,
            self.hi * b.lo,
            self.hi * b.hi,
        ];
        if p.iter().any(|v| !v.is_finite()) {
            return Self::BAD;
        }
        Self::out(
            p.iter().copied().fold(f64::INFINITY, f64::min),
            p.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        )
    }
    fn div(self, b: Self) -> Self {
        if !self.ok() || !b.ok() || b.lo <= 0. && b.hi >= 0. {
            return Self::BAD;
        }
        if self.zero() {
            return Self::p(0.);
        }
        self.mul(Self::out(1. / b.hi, 1. / b.lo))
    }
    fn sq(self) -> Self {
        if !self.ok() {
            return Self::BAD;
        }
        if self.zero() {
            return self;
        }
        let hi = self.lo.abs().max(self.hi.abs());
        let lo = if self.lo <= 0. && self.hi >= 0. {
            0.
        } else {
            self.lo.abs().min(self.hi.abs())
        };
        let hi = hi * hi;
        let lo = lo * lo;
        if !hi.is_finite() {
            return Self::BAD;
        }
        Self::bounds(if lo == 0. { 0. } else { down(lo).max(0.) }, up(hi))
    }
    fn sqrt(self) -> Self {
        if !self.ok() || self.lo < 0. {
            return Self::BAD;
        }
        if self.zero() {
            return self;
        }
        Self::bounds(
            if self.lo == 0. {
                0.
            } else {
                down(self.lo.sqrt()).max(0.)
            },
            up(self.hi.sqrt()),
        )
    }
    fn absmax(self) -> f64 {
        if self.ok() {
            self.lo.abs().max(self.hi.abs())
        } else {
            f64::INFINITY
        }
    }
    fn mid(self) -> f64 {
        if !self.ok() {
            return f64::NAN;
        }
        (self.lo * 0.5 + self.hi * 0.5).clamp(self.lo, self.hi)
    }
    fn positive(self) -> bool {
        self.ok() && self.lo > 0.
    }
}
#[derive(Clone, Copy, Debug)]
struct V {
    x: I,
    y: I,
}
impl V {
    fn p(p: Point<UserSpace>) -> Self {
        Self {
            x: I::p(p.x()),
            y: I::p(p.y()),
        }
    }
    fn add(self, b: Self) -> Self {
        Self {
            x: self.x.add(b.x),
            y: self.y.add(b.y),
        }
    }
    fn sub(self, b: Self) -> Self {
        Self {
            x: self.x.sub(b.x),
            y: self.y.sub(b.y),
        }
    }
    fn scale(self, s: I) -> Self {
        Self {
            x: self.x.mul(s),
            y: self.y.mul(s),
        }
    }
    fn neg(self) -> Self {
        Self {
            x: self.x.neg(),
            y: self.y.neg(),
        }
    }
    fn left(self) -> Self {
        Self {
            x: self.y.neg(),
            y: self.x,
        }
    }
    fn dot(self, b: Self) -> I {
        self.x.mul(b.x).add(self.y.mul(b.y))
    }
    fn cross(self, b: Self) -> I {
        self.x.mul(b.y).sub(self.y.mul(b.x))
    }
    fn len(self) -> I {
        let mut squared = self.x.sq().add(self.y.sq());
        // Both exact operands are squares. Generic outward addition can step
        // a zero lower endpoint below zero; retain the proved nonnegative bound.
        if squared.ok() {
            squared.lo = squared.lo.max(0.);
        }
        squared.sqrt()
    }
    fn unit(self) -> Self {
        let l = self.len();
        if !l.positive() {
            return Self {
                x: I::BAD,
                y: I::BAD,
            };
        }
        Self {
            x: self.x.div(l),
            y: self.y.div(l),
        }
    }
    fn mid(self, b: Self) -> Self {
        self.add(b).scale(I::p(0.5))
    }
    fn ok(self) -> bool {
        self.x.ok() && self.y.ok()
    }
}
#[derive(Clone, Copy)]
struct Vertex {
    ideal: V,
    p: Point<UserSpace>,
}
#[derive(Clone, Copy)]
struct Section {
    center: V,
    normal: V,
    minus: Vertex,
    plus: Vertex,
}
#[derive(Clone, Copy)]
struct Segment {
    start: Section,
    end: Section,
}
struct Builder<'a> {
    b: &'a mut Budget,
    map: Affine<UserSpace, DeviceSpace>,
    radius: I,
    linear: I,
    style: StrokeStyle,
    out: Vec<PathCommand>,
}
impl Builder<'_> {
    fn mapped(&self, p: V) -> V {
        let [a, b, c, d, e, f] = self.map.coefficients();
        V {
            x: I::p(a).mul(p.x).add(I::p(c).mul(p.y)).add(I::p(e)),
            y: I::p(b).mul(p.x).add(I::p(d).mul(p.y)).add(I::p(f)),
        }
    }
    fn vertex(&mut self, ideal: V) -> Result<Vertex, StrokeError> {
        self.b.work(16)?;
        if !ideal.ok() || ideal.x.absmax() > USER_MAX || ideal.y.absmax() > USER_MAX {
            return Err(self
                .b
                .error(StrokeErrorKind::Arithmetic("user vertex enclosure")));
        }
        let p = Point::new(ideal.x.mid(), ideal.y.mid())
            .map_err(|e| self.b.error(StrokeErrorKind::Geometry(e)))?;
        let mapped = self
            .map
            .transform_point(p)
            .map_err(|e| self.b.error(StrokeErrorKind::Geometry(e)))?;
        let bounds = self.mapped(ideal);
        if !bounds.ok() || bounds.x.absmax() > DEVICE_MAX || bounds.y.absmax() > DEVICE_MAX {
            return Err(self
                .b
                .error(StrokeErrorKind::Limit("device coordinate magnitude")));
        }
        let error = V {
            x: bounds.x.sub(I::p(mapped.x())),
            y: bounds.y.sub(I::p(mapped.y())),
        }
        .len();
        if !error.ok() || error.hi > EPS {
            return Err(self
                .b
                .error(StrokeErrorKind::Arithmetic("numeric vertex error")));
        }
        Ok(Vertex { ideal, p })
    }
    fn section(&mut self, center: V, normal: V) -> Result<Section, StrokeError> {
        if !normal.ok() {
            return Err(self
                .b
                .error(StrokeErrorKind::Unsupported("zero or unresolved tangent")));
        }
        let offset = normal.scale(self.radius);
        Ok(Section {
            center,
            normal,
            minus: self.vertex(center.sub(offset))?,
            plus: self.vertex(center.add(offset))?,
        })
    }
    fn orientation(points: &[Vertex]) -> Option<bool> {
        let mut sign = None;
        for i in 0..points.len() {
            let a = points[i].ideal;
            let b = points[(i + 1) % points.len()].ideal;
            let c = points[(i + 2) % points.len()].ideal;
            let cross = b.sub(a).cross(c.sub(b));
            let s = if cross.positive() {
                true
            } else if cross.ok() && cross.hi < 0. {
                false
            } else {
                return None;
            };
            if sign.is_some_and(|old| old != s) {
                return None;
            }
            sign = Some(s);
        }
        sign
    }
    fn component(&mut self, points: &[Vertex]) -> Result<(), StrokeError> {
        self.b.work(points.len() as u64 * 8)?;
        let forward = Self::orientation(points).ok_or_else(|| {
            self.b
                .error(StrokeErrorKind::Arithmetic("unresolved convex component"))
        })?;
        self.b.count(Counter::Components, 1)?;
        self.b.count(Counter::Commands, points.len() + 1)?;
        self.b.count(Counter::Points, points.len())?;
        let end = self
            .out
            .len()
            .checked_add(points.len() + 1)
            .ok_or_else(|| self.b.error(StrokeErrorKind::Arithmetic("command size")))?;
        if end > self.out.capacity() {
            let cap = self
                .out
                .capacity()
                .saturating_mul(2)
                .min(self.b.limits.max_output_commands)
                .max(end);
            let bytes = cap
                .checked_mul(size_of::<PathCommand>())
                .ok_or_else(|| self.b.error(StrokeErrorKind::Arithmetic("allocation size")))?;
            self.b.count(Counter::Bytes, bytes)?;
            self.out
                .try_reserve_exact(cap - self.out.len())
                .map_err(|_| self.b.error(StrokeErrorKind::Allocation))?;
        }
        for i in 0..points.len() {
            let p = points[if forward { i } else { points.len() - 1 - i }].p;
            self.out.push(if i == 0 {
                PathCommand::MoveTo(p)
            } else {
                PathCommand::LineTo(p)
            });
        }
        self.out.push(PathCommand::ClosePath);
        Ok(())
    }
    fn line(&mut self, a: V, z: V) -> Result<Segment, StrokeError> {
        self.b.count(Counter::Lines, 1)?;
        self.b.work(16)?;
        let normal = z.sub(a).unit().left();
        let start = self.section(a, normal)?;
        let end = self.section(z, normal)?;
        self.component(&[start.minus, end.minus, end.plus, start.plus])?;
        Ok(Segment { start, end })
    }
    fn arc(
        &mut self,
        center: V,
        u: V,
        v: V,
        endpoints: [Vertex; 2],
        known_quarter: bool,
    ) -> Result<(), StrokeError> {
        #[derive(Clone, Copy)]
        struct ArcNode {
            u: V,
            v: V,
            a: Vertex,
            z: Vertex,
            depth: usize,
            quarter: bool,
        }
        let [a, z] = endpoints;
        let c = self.vertex(center)?;
        let mut stack = [None; DEPTH + 1];
        stack[0] = Some(ArcNode {
            u,
            v,
            a,
            z,
            depth: 0,
            quarter: known_quarter,
        });
        let mut count = 1;
        while count > 0 {
            count -= 1;
            let n = stack[count].take().expect("arc stack");
            self.b.count(Counter::ArcNodes, 1)?;
            self.b.work(24)?;
            let dot = n.u.dot(n.v);
            let quarter = n.quarter || dot.ok() && dot.lo >= 0.;
            let error =
                n.u.sub(n.v)
                    .dot(n.u.sub(n.v))
                    .mul(self.radius)
                    .mul(self.linear)
                    .mul(I::p(0.25));
            if quarter && error.ok() && error.hi <= EPS {
                self.b.count(Counter::ArcVertices, 1)?;
                self.component(&[c, n.a, n.z])?;
                continue;
            }
            if n.depth >= self.b.limits.max_arc_depth.min(DEPTH) {
                return Err(self
                    .b
                    .error(StrokeErrorKind::Limit("arc subdivision depth")));
            }
            let mid = n.u.add(n.v).unit();
            if !mid.ok() {
                return Err(self.b.error(StrokeErrorKind::Arithmetic("arc bisector")));
            }
            let m = self.vertex(center.add(mid.scale(self.radius)))?;
            stack[count] = Some(ArcNode {
                u: mid,
                v: n.v,
                a: m,
                z: n.z,
                depth: n.depth + 1,
                quarter,
            });
            stack[count + 1] = Some(ArcNode {
                u: n.u,
                v: mid,
                a: n.a,
                z: m,
                depth: n.depth + 1,
                quarter,
            });
            count += 2;
        }
        Ok(())
    }
    fn cap(&mut self, s: Section, start: bool) -> Result<(), StrokeError> {
        self.b.count(Counter::Caps, 1)?;
        self.b.work(8)?;
        let t = s.normal.left().neg();
        let outward = if start { t.neg() } else { t };
        match self.style.cap() {
            LineCap::Butt => Ok(()),
            LineCap::Square => {
                let extension = outward.scale(self.radius);
                let a = self.vertex(s.minus.ideal.add(extension))?;
                let z = self.vertex(s.plus.ideal.add(extension))?;
                self.component(&[s.minus, a, z, s.plus])
            }
            LineCap::Round => {
                let tip = self.vertex(s.center.add(outward.scale(self.radius)))?;
                self.arc(s.center, s.normal, outward, [s.plus, tip], true)?;
                self.arc(s.center, outward, s.normal.neg(), [tip, s.minus], true)
            }
        }
    }
    fn join(&mut self, a: Section, z: Section) -> Result<(), StrokeError> {
        self.b.count(Counter::Joins, 1)?;
        self.b.work(32)?;
        let turn = a.normal.cross(z.normal);
        let dot = a.normal.dot(z.normal);
        if turn.zero() && dot.positive() {
            return Ok(());
        }
        let left = if turn.positive() {
            true
        } else if turn.ok() && turn.hi < 0. {
            false
        } else {
            return Err(self
                .b
                .error(StrokeErrorKind::Unsupported("unresolved join or reversal")));
        };
        let u = if left { a.normal.neg() } else { a.normal };
        let v = if left { z.normal.neg() } else { z.normal };
        let center = self.vertex(a.center)?;
        let p = if left { a.minus } else { a.plus };
        let q = if left { z.minus } else { z.plus };
        match self.style.join() {
            LineJoin::Round => self.arc(a.center, u, v, [p, q], false),
            LineJoin::Bevel => self.component(&[center, p, q]),
            LineJoin::Miter => {
                let denominator = I::p(1.).add(u.dot(v));
                if !denominator.positive() {
                    return Err(self.b.error(StrokeErrorKind::Unsupported("miter reversal")));
                }
                let m = u.add(v).scale(self.radius.div(denominator));
                let ratio = m.len().div(self.radius);
                let limit = self.style.miter_limit();
                if ratio.ok() && ratio.lo > limit {
                    self.component(&[center, p, q])
                } else if ratio.ok() && ratio.hi <= limit {
                    let m = self.vertex(a.center.add(m))?;
                    self.component(&[center, p, m, q])
                } else {
                    Err(self.b.error(StrokeErrorKind::Arithmetic("miter threshold")))
                }
            }
        }
    }
}
#[derive(Clone, Copy)]
struct CurveNode {
    p: [V; 4],
    start: Section,
    end: Section,
    depth: usize,
}
impl Builder<'_> {
    fn curve_admitted(&self, n: CurveNode) -> bool {
        let e = n.p[3].sub(n.p[0]).unit();
        if !e.ok() {
            return false;
        }
        let d = [
            n.p[1].sub(n.p[0]).scale(I::p(3.)),
            n.p[2].sub(n.p[1]).scale(I::p(3.)),
            n.p[3].sub(n.p[2]).scale(I::p(3.)),
        ];
        let mut lower = f64::INFINITY;
        let mut cross = 0f64;
        for v in d {
            let projection = v.dot(e);
            if !projection.positive() {
                return false;
            }
            lower = lower.min(projection.lo);
            cross = cross.max(v.cross(e).absmax());
        }
        let angle = I::p(cross).div(I::p(lower));
        let tangent = angle.mul(self.radius).mul(self.linear).mul(I::p(2.));
        let residual = n.p[1]
            .sub(n.p[0])
            .cross(e)
            .absmax()
            .max(n.p[2].sub(n.p[0]).cross(e).absmax());
        let center = I::p(residual).mul(self.linear);
        if !tangent.ok() || tangent.hi > EPS || !center.ok() || center.hi > EPS {
            return false;
        }
        let a = [
            d[1].sub(d[0]).scale(I::p(2.)),
            d[2].sub(d[1]).scale(I::p(2.)),
        ];
        let mut upper = 0f64;
        for v in d {
            for w in a {
                upper = upper.max(v.cross(w).absmax());
            }
        }
        let l = I::p(lower);
        let fold = I::p(upper).mul(self.radius).div(l.mul(l).mul(l));
        fold.ok()
            && fold.hi <= 0.5
            && Self::orientation(&[n.start.minus, n.end.minus, n.end.plus, n.start.plus]).is_some()
    }
    fn cubic(&mut self, p: [V; 4]) -> Result<Segment, StrokeError> {
        self.b.work(32)?;
        let start = self.section(p[0], p[1].sub(p[0]).unit().left())?;
        let end = self.section(p[3], p[3].sub(p[2]).unit().left())?;
        let mut stack = [None; DEPTH + 1];
        stack[0] = Some(CurveNode {
            p,
            start,
            end,
            depth: 0,
        });
        let mut count = 1;
        while count > 0 {
            count -= 1;
            let n = stack[count].take().expect("curve stack");
            self.b.count(Counter::CurveNodes, 1)?;
            self.b.work(128)?;
            if self.curve_admitted(n) {
                self.b.count(Counter::CurveLeaves, 1)?;
                self.component(&[n.start.minus, n.end.minus, n.end.plus, n.start.plus])?;
                continue;
            }
            if n.depth >= self.b.limits.max_subdivision_depth.min(DEPTH) {
                return Err(self
                    .b
                    .error(StrokeErrorKind::Limit("curve subdivision depth")));
            }
            let a = n.p[0].mid(n.p[1]);
            let b = n.p[1].mid(n.p[2]);
            let c = n.p[2].mid(n.p[3]);
            let j = a.mid(b);
            let k = b.mid(c);
            let m = j.mid(k);
            // One shared ideal normal and emitted cross section for both children.
            let section = self.section(m, k.sub(j).unit().left())?;
            stack[count] = Some(CurveNode {
                p: [m, k, c, n.p[3]],
                start: section,
                end: n.end,
                depth: n.depth + 1,
            });
            stack[count + 1] = Some(CurveNode {
                p: [n.p[0], a, j, m],
                start: n.start,
                end: section,
                depth: n.depth + 1,
            });
            count += 2;
        }
        Ok(Segment { start, end })
    }
    fn finish_open(
        &mut self,
        first: Option<Segment>,
        last: Option<Segment>,
    ) -> Result<(), StrokeError> {
        if let (Some(first), Some(last)) = (first, last) {
            self.cap(first.start, true)?;
            self.cap(last.end, false)?;
        }
        Ok(())
    }
}
fn input(p: Point<UserSpace>, b: &Budget) -> Result<V, StrokeError> {
    if p.x().abs() > USER_MAX || p.y().abs() > USER_MAX {
        return Err(b.error(StrokeErrorKind::Limit("user coordinate magnitude")));
    }
    Ok(V::p(p))
}
fn build(
    commands: &[PathCommand],
    style: StrokeStyle,
    map: Affine<UserSpace, DeviceSpace>,
    b: &mut Budget,
) -> Result<StrokeOutline, StrokeError> {
    b.count(Counter::Input, commands.len())?;
    b.work(commands.len() as u64 * 4)?;
    let [a, bb, c, d, _, _] = map.coefficients();
    let determinant = I::p(a).mul(I::p(d)).sub(I::p(bb).mul(I::p(c)));
    if !determinant.ok() || determinant.lo <= 0. && determinant.hi >= 0. {
        return Err(b.error(StrokeErrorKind::Arithmetic(
            "singular or unresolved transform",
        )));
    }
    let linear = I::p(a)
        .sq()
        .add(I::p(bb).sq())
        .add(I::p(c).sq())
        .add(I::p(d).sq())
        .sqrt();
    let radius = I::p(style.width()).mul(I::p(0.5));
    if !linear.positive() || !radius.positive() {
        return Err(b.error(StrokeErrorKind::Arithmetic("radius or transform underflow")));
    }
    let mut builder = Builder {
        b,
        map,
        radius,
        linear,
        style,
        out: Vec::new(),
    };
    let mut current = None;
    let mut beginning = None;
    let mut first = None;
    let mut last: Option<Segment> = None;
    let mut closed = false;
    for (index, command) in commands.iter().enumerate() {
        builder.b.index = Some(index);
        match *command {
            PathCommand::MoveTo(p) => {
                builder.finish_open(first, last)?;
                builder.b.count(Counter::Subpaths, 1)?;
                let p = input(p, builder.b)?;
                current = Some(p);
                beginning = Some(p);
                first = None;
                last = None;
                closed = false;
            }
            PathCommand::LineTo(p) => {
                if closed {
                    return Err(builder.b.error(StrokeErrorKind::Unsupported(
                        "drawing after close without move",
                    )));
                }
                let a = current.ok_or_else(|| {
                    builder
                        .b
                        .error(StrokeErrorKind::Invariant("line without move"))
                })?;
                let p = input(p, builder.b)?;
                let segment = builder.line(a, p)?;
                if let Some(previous) = last {
                    builder.join(previous.end, segment.start)?;
                }
                first.get_or_insert(segment);
                last = Some(segment);
                current = Some(p);
            }
            PathCommand::CubicTo {
                control1,
                control2,
                end,
            } => {
                if closed {
                    return Err(builder.b.error(StrokeErrorKind::Unsupported(
                        "drawing after close without move",
                    )));
                }
                let a = current.ok_or_else(|| {
                    builder
                        .b
                        .error(StrokeErrorKind::Invariant("curve without move"))
                })?;
                let end = input(end, builder.b)?;
                let p = [
                    a,
                    input(control1, builder.b)?,
                    input(control2, builder.b)?,
                    end,
                ];
                let segment = builder.cubic(p)?;
                if let Some(previous) = last {
                    builder.join(previous.end, segment.start)?;
                }
                first.get_or_insert(segment);
                last = Some(segment);
                current = Some(end);
            }
            PathCommand::ClosePath => {
                if closed {
                    continue;
                }
                let a = current.ok_or_else(|| {
                    builder
                        .b
                        .error(StrokeErrorKind::Invariant("close without move"))
                })?;
                let start = beginning.expect("current subpath");
                let Some(initial) = first else {
                    return Err(builder
                        .b
                        .error(StrokeErrorKind::Unsupported("degenerate closed stroke")));
                };
                let previous = last.expect("first segment");
                // Exact original endpoints can coincide without inventing a zero segment.
                if a.x == start.x && a.y == start.y {
                    builder.join(previous.end, initial.start)?;
                } else {
                    let closing = builder.line(a, start)?;
                    builder.join(previous.end, closing.start)?;
                    builder.join(closing.end, initial.start)?;
                }
                current = Some(start);
                first = None;
                last = None;
                closed = true;
            }
        }
    }
    builder.finish_open(first, last)?;
    Ok(StrokeOutline {
        commands: builder.out,
    })
}

#[cfg(test)]
mod interval_tests {
    use super::*;
    // Exact dyadic conversion for the bounded normal numbers in these tests.
    fn fraction(v: f64) -> (i128, i128) {
        if v == 0. {
            return (0, 1);
        }
        let bits = v.to_bits();
        let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
        let sign = if v < 0. { -1 } else { 1 };
        let n = i128::from((bits & ((1u64 << 52) - 1)) | (1u64 << 52)) * sign;
        if exponent >= 0 {
            (n << exponent, 1)
        } else {
            (n, 1i128 << -exponent)
        }
    }
    fn contains_rational(interval: I, numerator: i128, denominator: i128) {
        let (numerator, denominator) = if denominator < 0 {
            (-numerator, -denominator)
        } else {
            (numerator, denominator)
        };
        let (lo, ld) = fraction(interval.lo);
        let (hi, hd) = fraction(interval.hi);
        assert!(lo * denominator <= numerator * ld);
        assert!(hi * denominator >= numerator * hd);
    }
    #[test]
    fn signed_zero_and_subnormal_neighbors() {
        assert_eq!(up(-0.).to_bits(), 1);
        assert_eq!(down(0.).to_bits(), (1u64 << 63) | 1);
        assert_eq!(up(-f64::from_bits(1)), -0.);
        assert_eq!(down(f64::from_bits(1)), 0.);
    }
    #[test]
    fn dyadic_operations_enclose_exact_integer_results() {
        for a in -64..=64 {
            for b in -64..=64 {
                let x = I::p(f64::from(a) / 8.);
                let y = I::p(f64::from(b) / 8.);
                let sum = x.add(y);
                if a + b == 0 {
                    assert!(sum.lo <= 0. && sum.hi >= 0.);
                } else {
                    contains_rational(sum, i128::from(a + b), 8);
                }
                let mul = x.mul(y);
                contains_rational(mul, i128::from(a * b), 64);
                if b != 0 {
                    let div = x.div(y);
                    contains_rational(div, i128::from(a), i128::from(b));
                }
            }
        }
    }
    #[test]
    fn underflow_cannot_authorize_positive_denominator() {
        let tiny = I::p(f64::from_bits(1)).mul(I::p(0.5));
        assert!(!tiny.positive());
        assert!(!I::p(1.).div(tiny).ok());
        assert!(!I::p(f64::MAX).mul(I::p(2.)).ok());
    }
    #[test]
    fn root_endpoints_bracket_exact_integer_squares() {
        for i in 1..10000 {
            let root = I::p(f64::from(i)).sqrt();
            let (lo, ld) = fraction(root.lo);
            let (hi, hd) = fraction(root.hi);
            assert!(lo * lo <= i128::from(i) * ld * ld);
            assert!(hi * hi >= i128::from(i) * hd * hd);
        }
        assert!(!I::p(-1.).sqrt().ok());
    }
    #[test]
    fn emitted_midpoints_remain_inside_subnormal_and_distant_boxes() {
        for value in [f64::from_bits(1), -f64::from_bits(1), 1e12, -1e12] {
            assert_eq!(I::p(value).mid(), value);
            let interval = I::bounds(down(value), up(value));
            assert!(interval.mid() >= interval.lo && interval.mid() <= interval.hi);
        }
    }
    #[test]
    fn carried_midpoint_uncertainty_is_not_discarded() {
        let a = I::p(1e12);
        let b = I::p(1e12 + 0.001);
        let split = a.add(b).mul(I::p(0.5));
        assert!(split.lo < split.hi);
        let child = split.add(a).mul(I::p(0.5));
        assert!(child.hi - child.lo >= f64::EPSILON);
    }
}
