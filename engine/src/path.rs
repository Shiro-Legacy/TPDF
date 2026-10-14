// SPDX-License-Identifier: AGPL-3.0-or-later
// State/paint ordering references MuPDF1.28.1 pdf-op-run.c and fitz/path.c,
// Copyright (C) 2004-2026 Artifex Software, Inc. See NOTICE.
//! Strict standalone path display lists. No rasterization, font/resource lookup,
//! color management, or mixed-transform path construction. All arenas are flat.
use crate::content::{ContentError, ContentLimits, ContentOperand, ContentParser};
use crate::geometry::{Affine, GeometryError, PageSpace, Point, Rect, UserSpace};
use crate::operators::{OperatorError, OperatorKind, ValidatedOperation, validate_operation};
use std::{fmt, mem::size_of, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InitialClip {
    Unbounded,
    Rectangle(Rect<PageSpace>),
    Empty,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathInitialState {
    ctm: Affine<UserSpace, PageSpace>,
    clip: InitialClip,
}
impl PathInitialState {
    pub fn new(ctm: Affine<UserSpace, PageSpace>, clip: InitialClip) -> Self {
        Self { ctm, clip }
    }
}
impl Default for PathInitialState {
    fn default() -> Self {
        Self::new(
            Affine::from_pdf([1., 0., 0., 1., 0., 0.]).expect("finite identity"),
            InitialClip::Unbounded,
        )
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PathLimits {
    pub max_operations: usize,
    pub max_save_depth: usize,
    pub max_current_commands: usize,
    pub max_total_commands: usize,
    pub max_paths: usize,
    pub max_events: usize,
    pub max_clips: usize,
    pub max_dash_elements: usize,
    pub max_total_dash_elements: usize,
    /// Cumulative full requested capacities, including every growth and discarded
    /// buffer. No refunds. This is stricter than the original P0 growth-only cap.
    pub max_allocated_bytes: usize,
}
impl Default for PathLimits {
    fn default() -> Self {
        Self {
            max_operations: 1_000_000,
            max_save_depth: 128,
            max_current_commands: 100_000,
            max_total_commands: 1_000_000,
            max_paths: 100_000,
            max_events: 300_000,
            max_clips: 100_000,
            max_dash_elements: 256,
            max_total_dash_elements: 100_000,
            max_allocated_bytes: 64 * 1024 * 1024,
        }
    }
}
macro_rules! id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct $name(usize);
        impl $name {
            pub fn index(self) -> usize {
                self.0
            }
        }
    };
}
id!(PathId);
id!(ClipId);
id!(DashId);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeviceColor {
    Gray(f64),
    Rgb([f64; 3]),
    Cmyk([f64; 4]),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStyle {
    width: f64,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f64,
    dash: Option<DashId>,
    dash_phase: f64,
}
impl StrokeStyle {
    pub fn width(self) -> f64 {
        self.width
    }
    pub fn cap(self) -> LineCap {
        self.cap
    }
    pub fn join(self) -> LineJoin {
        self.join
    }
    pub fn miter_limit(self) -> f64 {
        self.miter_limit
    }
    pub fn dash(self) -> Option<DashId> {
        self.dash
    }
    pub fn dash_phase(self) -> f64 {
        self.dash_phase
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCommand {
    MoveTo(Point<UserSpace>),
    LineTo(Point<UserSpace>),
    CubicTo {
        control1: Point<UserSpace>,
        control2: Point<UserSpace>,
        end: Point<UserSpace>,
    },
    ClosePath,
}
#[derive(Debug, PartialEq)]
pub struct OwnedPath {
    commands: Vec<PathCommand>,
}
impl OwnedPath {
    pub fn commands(&self) -> &[PathCommand] {
        &self.commands
    }
}
#[derive(Debug, PartialEq)]
pub struct DashPattern {
    values: Vec<f64>,
}
impl DashPattern {
    pub fn values(&self) -> &[f64] {
        &self.values
    }
}
#[derive(Debug, PartialEq)]
pub struct PaintEvent {
    path: PathId,
    ctm: Affine<UserSpace, PageSpace>,
    color: DeviceColor,
    clip: Option<ClipId>,
    span: Range<usize>,
}
impl PaintEvent {
    pub fn path(&self) -> PathId {
        self.path
    }
    pub fn ctm(&self) -> Affine<UserSpace, PageSpace> {
        self.ctm
    }
    pub fn color(&self) -> DeviceColor {
        self.color
    }
    pub fn clip(&self) -> Option<ClipId> {
        self.clip
    }
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }
}
#[derive(Debug, PartialEq)]
pub enum PathEvent {
    Fill {
        paint: PaintEvent,
        rule: FillRule,
    },
    Stroke {
        paint: PaintEvent,
        style: StrokeStyle,
    },
    ClipPush {
        id: ClipId,
        span: Range<usize>,
    },
    ClipRestore {
        clip: Option<ClipId>,
        span: Range<usize>,
    },
}
#[derive(Debug, PartialEq)]
pub struct ClipNode {
    parent: Option<ClipId>,
    path: PathId,
    rule: FillRule,
    ctm: Affine<UserSpace, PageSpace>,
}
impl ClipNode {
    pub fn parent(&self) -> Option<ClipId> {
        self.parent
    }
    pub fn path(&self) -> PathId {
        self.path
    }
    pub fn rule(&self) -> FillRule {
        self.rule
    }
    pub fn ctm(&self) -> Affine<UserSpace, PageSpace> {
        self.ctm
    }
}
#[derive(Debug, PartialEq)]
pub struct PathProgram {
    initial: PathInitialState,
    paths: Vec<OwnedPath>,
    events: Vec<PathEvent>,
    clips: Vec<ClipNode>,
    dashes: Vec<DashPattern>,
}
impl PathProgram {
    pub fn initial_clip(&self) -> InitialClip {
        self.initial.clip
    }
    pub fn initial_ctm(&self) -> Affine<UserSpace, PageSpace> {
        self.initial.ctm
    }
    pub fn paths(&self) -> &[OwnedPath] {
        &self.paths
    }
    pub fn events(&self) -> &[PathEvent] {
        &self.events
    }
    pub fn clips(&self) -> &[ClipNode] {
        &self.clips
    }
    pub fn dashes(&self) -> &[DashPattern] {
        &self.dashes
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct PathError {
    pub span: Range<usize>,
    pub kind: PathErrorKind,
}
#[derive(Debug, Clone, PartialEq)]
pub enum PathErrorKind {
    Content(ContentError),
    Operator(OperatorError),
    Unsupported(OperatorKind),
    MixedPathTransformUnsupported,
    InexactInteger,
    ColorRangeUnsupported,
    MissingCurrentPoint,
    StackUnderflow,
    UnbalancedState,
    UnfinishedPath,
    Geometry(GeometryError),
    LimitExceeded(&'static str),
    AllocationFailed,
    Failed,
}
impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PDF path execution at bytes {}..{}: {:?}",
            self.span.start, self.span.end, self.kind
        )
    }
}
impl std::error::Error for PathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            PathErrorKind::Content(e) => Some(e),
            PathErrorKind::Operator(e) => Some(e),
            PathErrorKind::Geometry(e) => Some(e),
            _ => None,
        }
    }
}
#[derive(Clone, Copy)]
struct State {
    ctm: Affine<UserSpace, PageSpace>,
    stroke: StrokeStyle,
    fill: DeviceColor,
    stroke_color: DeviceColor,
    clip: Option<ClipId>,
    /// Components of the current fill/stroke color space (1, 3 or 4).
    fill_space: usize,
    stroke_space: usize,
}
pub struct PathInterpreter {
    program: PathProgram,
    limits: PathLimits,
    state: State,
    stack: Vec<State>,
    current: Vec<PathCommand>,
    point: Option<Point<UserSpace>>,
    start: Option<Point<UserSpace>>,
    closed: bool,
    pending_clip: Option<FillRule>,
    operations: usize,
    total_commands: usize,
    total_dash: usize,
    allocated: usize,
    failed: bool,
    span: Range<usize>,
    /// Component count the caller resolved for the next cs/CS operation.
    pub(crate) pending_space: Option<usize>,
}
fn reserve<T>(
    values: &mut Vec<T>,
    add: usize,
    allocated: &mut usize,
    limit: usize,
) -> Result<(), PathErrorKind> {
    let end = values
        .len()
        .checked_add(add)
        .ok_or(PathErrorKind::LimitExceeded("allocation"))?;
    if end <= values.capacity() {
        return Ok(());
    }
    let ceiling = limit.saturating_sub(*allocated) / size_of::<T>().max(1);
    if end > ceiling {
        return Err(PathErrorKind::LimitExceeded("allocated bytes"));
    }
    let target = end
        .max(values.capacity().saturating_mul(2).max(4))
        .min(ceiling);
    let requested = target
        .checked_mul(size_of::<T>())
        .ok_or(PathErrorKind::LimitExceeded("allocated bytes"))?;
    // Charge the full request before allocation, retaining attempted work even
    // if the allocator rejects it. The caller can temporarily lower `limit`.
    *allocated = allocated
        .checked_add(requested)
        .filter(|&v| v <= limit)
        .ok_or(PathErrorKind::LimitExceeded("allocated bytes"))?;
    values
        .try_reserve_exact(target - values.len())
        .map_err(|_| PathErrorKind::AllocationFailed)?;
    Ok(())
}
fn number(value: &ContentOperand) -> Result<f64, PathErrorKind> {
    match value {
        ContentOperand::Integer(v)
            if !(-9_007_199_254_740_992..=9_007_199_254_740_992).contains(v) =>
        {
            Err(PathErrorKind::InexactInteger)
        }
        ContentOperand::Integer(v) => Ok(*v as f64),
        ContentOperand::Real(v) => Ok(*v),
        _ => unreachable!("validated numeric operand"),
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PathSnapshot {
    pub ctm: Affine<UserSpace, PageSpace>,
    pub fill: DeviceColor,
    pub clip: Option<ClipId>,
    pub save_depth: usize,
    pub active: bool,
}
impl PathInterpreter {
    pub(crate) fn snapshot(&self) -> PathSnapshot {
        PathSnapshot {
            ctm: self.state.ctm,
            fill: self.state.fill,
            clip: self.state.clip,
            save_depth: self.stack.len(),
            active: self.active(),
        }
    }
    pub(crate) fn event_count(&self) -> usize {
        self.program.events.len()
    }
    /// Fill and stroke events paint; clip events only change state.
    pub(crate) fn paints(&self, event: usize) -> bool {
        matches!(
            self.program.events[event],
            PathEvent::Fill { .. } | PathEvent::Stroke { .. }
        )
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.allocated
    }
    pub(crate) fn event_upper_bound(&self, kind: OperatorKind) -> usize {
        use OperatorKind::*;
        let nonempty = usize::from(!self.current.is_empty());
        let clip = usize::from(self.pending_clip.is_some());
        match kind {
            FillStroke | EvenOddFillStroke | CloseFillStroke | CloseEvenOddFillStroke => {
                2 * nonempty + clip
            }
            Fill | EvenOddFill | Stroke | CloseStroke => nonempty + clip,
            EndPath => clip,
            Restore => usize::from(self.stack.last().is_some_and(|s| s.clip != self.state.clip)),
            _ => 0,
        }
    }
    pub(crate) fn push_budgeted(
        &mut self,
        operation: &ValidatedOperation<'_>,
        allowance: usize,
    ) -> Result<(), PathError> {
        let configured = self.limits.max_allocated_bytes;
        self.limits.max_allocated_bytes = configured.min(self.allocated.saturating_add(allowance));
        let result = self.push(operation);
        self.limits.max_allocated_bytes = configured;
        result
    }
    pub fn new(initial: PathInitialState, limits: PathLimits) -> Result<Self, PathError> {
        Ok(Self {
            program: PathProgram {
                initial,
                paths: Vec::new(),
                events: Vec::new(),
                clips: Vec::new(),
                dashes: Vec::new(),
            },
            limits,
            state: State {
                ctm: initial.ctm,
                stroke: StrokeStyle {
                    width: 1.,
                    cap: LineCap::Butt,
                    join: LineJoin::Miter,
                    miter_limit: 10.,
                    dash: None,
                    dash_phase: 0.,
                },
                fill: DeviceColor::Gray(0.),
                stroke_color: DeviceColor::Gray(0.),
                clip: None,
                fill_space: 1,
                stroke_space: 1,
            },
            stack: Vec::new(),
            current: Vec::new(),
            point: None,
            start: None,
            closed: false,
            pending_clip: None,
            operations: 0,
            total_commands: 0,
            total_dash: 0,
            allocated: 0,
            failed: false,
            span: 0..0,
            pending_space: None,
        })
    }
    pub fn push(&mut self, op: &ValidatedOperation<'_>) -> Result<(), PathError> {
        if self.failed {
            return Err(PathError {
                span: self.span.clone(),
                kind: PathErrorKind::Failed,
            });
        }
        self.span = op.operation().span.clone();
        let result = self.execute(op);
        result.map_err(|kind| {
            self.failed = true;
            PathError {
                span: self.span.clone(),
                kind,
            }
        })
    }
    pub fn finish(self) -> Result<PathProgram, PathError> {
        let kind = if self.failed {
            Some(PathErrorKind::Failed)
        } else if !self.stack.is_empty() {
            Some(PathErrorKind::UnbalancedState)
        } else if !self.current.is_empty() || self.pending_clip.is_some() {
            Some(PathErrorKind::UnfinishedPath)
        } else {
            None
        };
        if let Some(kind) = kind {
            Err(PathError {
                span: self.span,
                kind,
            })
        } else {
            Ok(self.program)
        }
    }
    fn active(&self) -> bool {
        !self.current.is_empty() || self.pending_clip.is_some()
    }
    fn command(&mut self, command: PathCommand) -> Result<(), PathErrorKind> {
        if self.current.len() >= self.limits.max_current_commands {
            return Err(PathErrorKind::LimitExceeded("current path commands"));
        }
        self.total_commands = self
            .total_commands
            .checked_add(1)
            .filter(|&v| v <= self.limits.max_total_commands)
            .ok_or(PathErrorKind::LimitExceeded("total path commands"))?;
        reserve(
            &mut self.current,
            1,
            &mut self.allocated,
            self.limits.max_allocated_bytes,
        )?;
        self.current.push(command);
        Ok(())
    }
    fn move_to(&mut self, p: Point<UserSpace>) -> Result<(), PathErrorKind> {
        self.command(PathCommand::MoveTo(p))?;
        self.point = Some(p);
        self.start = Some(p);
        self.closed = false;
        Ok(())
    }
    fn line_to(&mut self, p: Point<UserSpace>) -> Result<(), PathErrorKind> {
        self.point.ok_or(PathErrorKind::MissingCurrentPoint)?;
        self.command(PathCommand::LineTo(p))?;
        self.point = Some(p);
        self.closed = false;
        Ok(())
    }
    fn close(&mut self) -> Result<(), PathErrorKind> {
        let start = self.start.ok_or(PathErrorKind::MissingCurrentPoint)?;
        if !self.closed {
            self.command(PathCommand::ClosePath)?;
        }
        self.point = Some(start);
        self.closed = true;
        Ok(())
    }
    fn event(&mut self, event: PathEvent) -> Result<(), PathErrorKind> {
        if self.program.events.len() >= self.limits.max_events {
            return Err(PathErrorKind::LimitExceeded("events"));
        }
        reserve(
            &mut self.program.events,
            1,
            &mut self.allocated,
            self.limits.max_allocated_bytes,
        )?;
        self.program.events.push(event);
        Ok(())
    }
    fn paint(&self, path: PathId, color: DeviceColor) -> PaintEvent {
        PaintEvent {
            path,
            ctm: self.state.ctm,
            color,
            clip: self.state.clip,
            span: self.span.clone(),
        }
    }
    fn end_path(
        &mut self,
        close: bool,
        fill: Option<FillRule>,
        stroke: bool,
    ) -> Result<(), PathErrorKind> {
        if close {
            self.close()?;
        }
        if (fill.is_some() || stroke || self.pending_clip.is_some())
            && (!self.current.is_empty() || self.pending_clip.is_some())
        {
            // Paint/clip geometry must remain finite in the output coordinate
            // space. Check once per completed path, without rewriting points or
            // computing inaccurate curve/stroke bounds.
            for command in &self.current {
                let check = |point| {
                    self.state
                        .ctm
                        .transform_point(point)
                        .map(|_| ())
                        .map_err(PathErrorKind::Geometry)
                };
                match *command {
                    PathCommand::MoveTo(p) | PathCommand::LineTo(p) => check(p)?,
                    PathCommand::CubicTo {
                        control1,
                        control2,
                        end,
                    } => {
                        check(control1)?;
                        check(control2)?;
                        check(end)?;
                    }
                    PathCommand::ClosePath => {}
                }
            }
            if self.program.paths.len() >= self.limits.max_paths {
                return Err(PathErrorKind::LimitExceeded("stored paths"));
            }
            reserve(
                &mut self.program.paths,
                1,
                &mut self.allocated,
                self.limits.max_allocated_bytes,
            )?;
            let path = PathId(self.program.paths.len());
            let nonempty = !self.current.is_empty();
            self.program.paths.push(OwnedPath {
                commands: std::mem::take(&mut self.current),
            });
            if nonempty {
                if let Some(rule) = fill {
                    self.event(PathEvent::Fill {
                        paint: self.paint(path, self.state.fill),
                        rule,
                    })?;
                }
                if stroke {
                    self.event(PathEvent::Stroke {
                        paint: self.paint(path, self.state.stroke_color),
                        style: self.state.stroke,
                    })?;
                }
            }
            if let Some(rule) = self.pending_clip {
                if self.program.clips.len() >= self.limits.max_clips {
                    return Err(PathErrorKind::LimitExceeded("clip nodes"));
                }
                reserve(
                    &mut self.program.clips,
                    1,
                    &mut self.allocated,
                    self.limits.max_allocated_bytes,
                )?;
                let id = ClipId(self.program.clips.len());
                self.program.clips.push(ClipNode {
                    parent: self.state.clip,
                    path,
                    rule,
                    ctm: self.state.ctm,
                });
                self.event(PathEvent::ClipPush {
                    id,
                    span: self.span.clone(),
                })?;
                self.state.clip = Some(id);
            }
        }
        self.current.clear();
        self.point = None;
        self.start = None;
        self.closed = false;
        self.pending_clip = None;
        Ok(())
    }
    fn execute(&mut self, op: &ValidatedOperation<'_>) -> Result<(), PathErrorKind> {
        use OperatorKind::*;
        self.operations = self
            .operations
            .checked_add(1)
            .filter(|&v| v <= self.limits.max_operations)
            .ok_or(PathErrorKind::LimitExceeded("operations"))?;
        let a = op.operands();
        let n = |i| number(&a[i]);
        let point = |i| Point::new(n(i)?, n(i + 1)?).map_err(PathErrorKind::Geometry);
        match op.kind() {
            Save => {
                if self.stack.len() >= self.limits.max_save_depth {
                    return Err(PathErrorKind::LimitExceeded("save depth"));
                }
                reserve(
                    &mut self.stack,
                    1,
                    &mut self.allocated,
                    self.limits.max_allocated_bytes,
                )?;
                self.stack.push(self.state);
            }
            Restore => {
                let previous = *self.stack.last().ok_or(PathErrorKind::StackUnderflow)?;
                if self.active() && previous.ctm.coefficients() != self.state.ctm.coefficients() {
                    return Err(PathErrorKind::MixedPathTransformUnsupported);
                }
                if previous.clip != self.state.clip {
                    self.event(PathEvent::ClipRestore {
                        clip: previous.clip,
                        span: self.span.clone(),
                    })?;
                }
                self.stack.pop();
                self.state = previous;
            }
            Concat => {
                if self.active() {
                    return Err(PathErrorKind::MixedPathTransformUnsupported);
                }
                let m = Affine::<UserSpace, UserSpace>::from_pdf([
                    n(0)?,
                    n(1)?,
                    n(2)?,
                    n(3)?,
                    n(4)?,
                    n(5)?,
                ])
                .map_err(PathErrorKind::Geometry)?;
                self.state.ctm = m.then(self.state.ctm).map_err(PathErrorKind::Geometry)?;
            }
            LineWidth => self.state.stroke.width = n(0)?,
            LineCap => {
                self.state.stroke.cap = match n(0)? as u8 {
                    0 => crate::path::LineCap::Butt,
                    1 => crate::path::LineCap::Round,
                    _ => crate::path::LineCap::Square,
                }
            }
            LineJoin => {
                self.state.stroke.join = match n(0)? as u8 {
                    0 => crate::path::LineJoin::Miter,
                    1 => crate::path::LineJoin::Round,
                    _ => crate::path::LineJoin::Bevel,
                }
            }
            MiterLimit => self.state.stroke.miter_limit = n(0)?,
            Dash => {
                let ContentOperand::Array(items) = &a[0] else {
                    unreachable!("validated dash")
                };
                if items.len() > self.limits.max_dash_elements {
                    return Err(PathErrorKind::LimitExceeded("dash elements"));
                }
                self.total_dash = self
                    .total_dash
                    .checked_add(items.len())
                    .filter(|&v| v <= self.limits.max_total_dash_elements)
                    .ok_or(PathErrorKind::LimitExceeded("total dash elements"))?;
                let phase = n(1)?;
                let mut values = Vec::new();
                reserve(
                    &mut values,
                    items.len(),
                    &mut self.allocated,
                    self.limits.max_allocated_bytes,
                )?;
                for item in items {
                    values.push(number(item)?);
                }
                self.state.stroke.dash = if values.is_empty() {
                    None
                } else {
                    reserve(
                        &mut self.program.dashes,
                        1,
                        &mut self.allocated,
                        self.limits.max_allocated_bytes,
                    )?;
                    let id = DashId(self.program.dashes.len());
                    self.program.dashes.push(DashPattern { values });
                    Some(id)
                };
                self.state.stroke.dash_phase = phase;
            }
            MoveTo => self.move_to(point(0)?)?,
            LineTo => self.line_to(point(0)?)?,
            CurveTo | CurveToV | CurveToY => {
                let current = self.point.ok_or(PathErrorKind::MissingCurrentPoint)?;
                let (control1, control2, end) = match op.kind() {
                    CurveTo => (point(0)?, point(2)?, point(4)?),
                    CurveToV => (current, point(0)?, point(2)?),
                    _ => (point(0)?, point(2)?, point(2)?),
                };
                self.command(PathCommand::CubicTo {
                    control1,
                    control2,
                    end,
                })?;
                self.point = Some(end);
                self.closed = false;
            }
            ClosePath => self.close()?,
            Rectangle => {
                let (x, y, w, h) = (n(0)?, n(1)?, n(2)?, n(3)?);
                let p = |x, y| Point::new(x, y).map_err(PathErrorKind::Geometry);
                self.move_to(p(x, y)?)?;
                self.line_to(p(x + w, y)?)?;
                self.line_to(p(x + w, y + h)?)?;
                self.line_to(p(x, y + h)?)?;
                self.close()?;
            }
            Stroke => self.end_path(false, None, true)?,
            CloseStroke => self.end_path(true, None, true)?,
            Fill => self.end_path(false, Some(FillRule::NonZero), false)?,
            EvenOddFill => self.end_path(false, Some(FillRule::EvenOdd), false)?,
            FillStroke => self.end_path(false, Some(FillRule::NonZero), true)?,
            EvenOddFillStroke => self.end_path(false, Some(FillRule::EvenOdd), true)?,
            CloseFillStroke => self.end_path(true, Some(FillRule::NonZero), true)?,
            CloseEvenOddFillStroke => self.end_path(true, Some(FillRule::EvenOdd), true)?,
            EndPath => self.end_path(false, None, false)?,
            Clip => self.pending_clip = Some(FillRule::NonZero),
            EvenOddClip => self.pending_clip = Some(FillRule::EvenOdd),
            // cs/CS select a device-equivalent space resolved by the caller;
            // its initial color is black.
            StrokeColorSpace | FillColorSpace => {
                let space = self
                    .pending_space
                    .take()
                    .ok_or(PathErrorKind::Unsupported(op.kind()))?;
                let black = match space {
                    1 => DeviceColor::Gray(0.),
                    3 => DeviceColor::Rgb([0.; 3]),
                    _ => DeviceColor::Cmyk([0., 0., 0., 1.]),
                };
                if op.kind() == StrokeColorSpace {
                    self.state.stroke_space = space;
                    self.state.stroke_color = black;
                } else {
                    self.state.fill_space = space;
                    self.state.fill = black;
                }
            }
            StrokeColor | FillColor | StrokeColorPattern | FillColorPattern | StrokeGray
            | FillGray | StrokeRgb | FillRgb | StrokeCmyk | FillCmyk => {
                let stroke = matches!(
                    op.kind(),
                    StrokeColor | StrokeColorPattern | StrokeGray | StrokeRgb | StrokeCmyk
                );
                let space = match op.kind() {
                    StrokeColor | StrokeColorPattern => self.state.stroke_space,
                    FillColor | FillColorPattern => self.state.fill_space,
                    _ => a.len(),
                };
                // A pattern name or a count that disagrees with the space refuses.
                if a.len() != space || a.iter().any(|v| matches!(v, ContentOperand::Name(_))) {
                    return Err(PathErrorKind::Unsupported(op.kind()));
                }
                let mut values = [0.; 4];
                for (i, slot) in values.iter_mut().enumerate().take(a.len()) {
                    *slot = n(i)?;
                    if !(0.0..=1.0).contains(slot) {
                        return Err(PathErrorKind::ColorRangeUnsupported);
                    }
                }
                let color = match a.len() {
                    1 => DeviceColor::Gray(values[0]),
                    3 => DeviceColor::Rgb([values[0], values[1], values[2]]),
                    _ => DeviceColor::Cmyk(values),
                };
                if stroke {
                    self.state.stroke_color = color;
                    self.state.stroke_space = space;
                } else {
                    self.state.fill = color;
                    self.state.fill_space = space;
                }
            }
            kind => return Err(PathErrorKind::Unsupported(kind)),
        }
        Ok(())
    }
}
pub fn interpret_path_content(
    bytes: &[u8],
    initial: PathInitialState,
    content_limits: ContentLimits,
    path_limits: PathLimits,
) -> Result<PathProgram, PathError> {
    let mut parser = ContentParser::new(bytes, content_limits);
    let mut interpreter = PathInterpreter::new(initial, path_limits)?;
    while let Some(op) = parser.next_operation().map_err(|e| PathError {
        span: e.offset..e.offset,
        kind: PathErrorKind::Content(e),
    })? {
        let validated = validate_operation(&op).map_err(|e| PathError {
            span: op.span.clone(),
            kind: PathErrorKind::Operator(e),
        })?;
        interpreter.push(&validated)?;
    }
    interpreter.finish()
}
