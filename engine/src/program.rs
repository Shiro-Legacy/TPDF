// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded mixed path and positioned-source-code traces, not page rendering.
//!
//! P0 owns graphics color/clip state; its finite CTM is checked against T0's
//! mirror at every operation. Both engines persist for the entire program.
//! Parsing and borrowed input/font storage are separately bounded prerequisites.
//! Execution charges cumulative full allocation requests before each reserve.
use crate::content::{
    ContentError, ContentLimits, ContentOperand as Value, ContentParser, Operation,
};
use crate::geometry::{Affine, PageSpace, Rect, UserSpace};
use crate::operators::{OperatorError, OperatorKind, ValidatedOperation, validate_operation};
use crate::path::{
    ClipId, DeviceColor, InitialClip, PathError, PathInitialState, PathInterpreter, PathLimits,
    PathProgram,
};
use crate::text_font::{TextFontId, TextFonts};
use crate::text_position::{
    PositionedText, TextPositionError, TextPositionInitialState, TextPositionLimits, TextPositioner,
};
use std::{error::Error, fmt, mem::size_of, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgramInitialState {
    ctm: Affine<UserSpace, PageSpace>,
    clip: InitialClip,
}
impl ProgramInitialState {
    pub fn new(ctm: Affine<UserSpace, PageSpace>, clip: InitialClip) -> Self {
        Self { ctm, clip }
    }
    pub fn ctm(self) -> Affine<UserSpace, PageSpace> {
        self.ctm
    }
    pub fn clip(self) -> InitialClip {
        self.clip
    }
}
impl Default for ProgramInitialState {
    fn default() -> Self {
        Self::new(
            TextPositionInitialState::default().ctm(),
            InitialClip::Unbounded,
        )
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ProgramLimits {
    pub path: PathLimits,
    pub text: TextPositionLimits,
    pub max_operations: usize,
    pub max_save_depth: usize,
    pub max_events: usize,
    pub max_text_contexts: usize,
    /// Reserved units: one per global operation, planned child dispatch, TJ or
    /// dash element, shown source byte, resolved font-name byte and typed Form
    /// dispatch/name/number and selected-font-table scan. Failed reservations
    /// are not refunded.
    pub max_work_units: usize,
    /// Full requested capacities for both children and coordinator buffers.
    /// The separately bounded parser and borrowed fonts/input are excluded.
    pub max_allocated_bytes: usize,
}
impl Default for ProgramLimits {
    fn default() -> Self {
        Self {
            path: PathLimits::default(),
            text: TextPositionLimits::default(),
            max_operations: 100_000,
            max_save_depth: 128,
            max_events: 600_000,
            max_text_contexts: 100_000,
            max_work_units: 4_000_000,
            max_allocated_bytes: 128 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProgramUsage {
    pub operations: usize,
    /// Actual child push calls, including calls that failed.
    pub child_operations: usize,
    pub work_units: usize,
    pub allocated_bytes: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramLimitKind {
    Operations,
    SaveDepth,
    Events,
    TextContexts,
    WorkUnits,
    AllocatedBytes,
}
#[derive(Clone, Debug, PartialEq)]
pub enum ProgramErrorKind {
    Content(Box<ContentError>),
    Operator(OperatorError),
    Path(Box<PathError>),
    Text(Box<TextPositionError>),
    Unsupported(OperatorKind),
    State(&'static str),
    Invariant(&'static str),
    InvalidSpan,
    LimitExceeded(ProgramLimitKind),
    AllocationFailed,
}
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramError {
    pub operation_index: usize,
    pub span: Range<usize>,
    pub kind: ProgramErrorKind,
}
impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "mixed program operation {} at {:?}: {:?}",
            self.operation_index, self.span, self.kind
        )
    }
}
impl Error for ProgramError {}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgramEventKind {
    Path(usize),
    Image(usize),
    Text { event: usize, context: usize },
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramEvent {
    operation_index: usize,
    span: Range<usize>,
    kind: ProgramEventKind,
}
impl ProgramEvent {
    pub fn operation_index(&self) -> usize {
        self.operation_index
    }
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }
    pub fn kind(&self) -> ProgramEventKind {
        self.kind
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextContext {
    ctm: Affine<UserSpace, PageSpace>,
    clip: Option<ClipId>,
    fill_color: DeviceColor,
}
impl TextContext {
    pub fn ctm(self) -> Affine<UserSpace, PageSpace> {
        self.ctm
    }
    pub fn clip(self) -> Option<ClipId> {
        self.clip
    }
    pub fn fill_color(self) -> DeviceColor {
        self.fill_color
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImagePaint {
    image_id: usize,
    ctm: Affine<UserSpace, PageSpace>,
    clip: Option<ClipId>,
    operation_index: usize,
}
impl ImagePaint {
    pub fn image_id(self) -> usize {
        self.image_id
    }
    pub fn ctm(self) -> Affine<UserSpace, PageSpace> {
        self.ctm
    }
    pub fn clip(self) -> Option<ClipId> {
        self.clip
    }
    pub fn operation_index(self) -> usize {
        self.operation_index
    }
}
#[derive(Debug)]
pub struct Program {
    paths: PathProgram,
    text: PositionedText,
    events: Vec<ProgramEvent>,
    text_contexts: Vec<TextContext>,
    images: Vec<ImagePaint>,
    usage: ProgramUsage,
}
impl Program {
    pub fn paths(&self) -> &PathProgram {
        &self.paths
    }
    pub fn text(&self) -> &PositionedText {
        &self.text
    }
    pub fn events(&self) -> &[ProgramEvent] {
        &self.events
    }
    pub fn images(&self) -> &[ImagePaint] {
        &self.images
    }
    pub fn text_contexts(&self) -> &[TextContext] {
        &self.text_contexts
    }
    pub fn usage(&self) -> ProgramUsage {
        self.usage
    }
}
type Failure = ProgramErrorKind;
/// Only an explicit selection of the PDF default intent is currently supported.
/// This is not a general color-management or arbitrary-ri no-op policy.
pub(crate) fn relative_colorimetric(operation: &ValidatedOperation<'_>) -> bool {
    operation.kind() == OperatorKind::RenderingIntent
        && matches!(&operation.operands()[0], Value::Name(name) if name == b"RelativeColorimetric")
}
pub(crate) fn optional_content(operation: &ValidatedOperation<'_>) -> bool {
    operation.kind() == OperatorKind::BeginMarkedContentProperties
        && matches!(&operation.operands()[0], Value::Name(name) if name == b"OC")
}
/// The page raster's fixed 1/64-device-pixel curve bound is tighter than
/// the admitted one-pixel tolerance. Zero requests that renderer's default.
/// Other tolerances await an explicit precision policy.
pub(crate) fn default_flatness(operation: &ValidatedOperation<'_>) -> bool {
    operation.kind() == OperatorKind::Flatness
        && match operation.operands()[0] {
            Value::Integer(0 | 1) => true,
            Value::Real(value) => value == 0. || value == 1.,
            _ => false,
        }
}
fn exceed(kind: ProgramLimitKind) -> Failure {
    Failure::LimitExceeded(kind)
}
fn count(
    current: usize,
    addition: usize,
    limit: usize,
    kind: ProgramLimitKind,
) -> Result<usize, Failure> {
    current
        .checked_add(addition)
        .filter(|&n| n <= limit)
        .ok_or_else(|| exceed(kind))
}
fn reserve<T>(
    values: &mut Vec<T>,
    additional: usize,
    max_items: usize,
    usage: &mut ProgramUsage,
    limit: usize,
) -> Result<(), Failure> {
    let end = values
        .len()
        .checked_add(additional)
        .filter(|&n| n <= max_items)
        .ok_or_else(|| exceed(ProgramLimitKind::AllocatedBytes))?;
    if end <= values.capacity() {
        return Ok(());
    }
    let target = values.capacity().saturating_mul(2).min(max_items).max(end);
    let requested = target
        .checked_mul(size_of::<T>())
        .ok_or_else(|| exceed(ProgramLimitKind::AllocatedBytes))?;
    usage.allocated_bytes = count(
        usage.allocated_bytes,
        requested,
        limit,
        ProgramLimitKind::AllocatedBytes,
    )?;
    values
        .try_reserve_exact(target - values.len())
        .map_err(|_| Failure::AllocationFailed)?;
    Ok(())
}
#[derive(Clone, Copy)]
enum Route {
    Shared,
    Path,
    Text,
}
fn route(kind: OperatorKind) -> Option<Route> {
    use OperatorKind::*;
    Some(match kind {
        Save | Restore | Concat => Route::Shared,
        LineWidth
        | LineCap
        | LineJoin
        | MiterLimit
        | Dash
        | MoveTo
        | LineTo
        | CurveTo
        | CurveToV
        | CurveToY
        | ClosePath
        | Rectangle
        | Stroke
        | CloseStroke
        | Fill
        | EvenOddFill
        | FillStroke
        | EvenOddFillStroke
        | CloseFillStroke
        | CloseEvenOddFillStroke
        | EndPath
        | Clip
        | EvenOddClip
        | StrokeGray
        | FillGray
        | StrokeRgb
        | FillRgb
        | StrokeCmyk
        | FillCmyk
        | StrokeColorSpace
        | FillColorSpace
        | StrokeColor
        | FillColor
        | StrokeColorPattern
        | FillColorPattern => Route::Path,
        BeginText
        | EndText
        | CharacterSpacing
        | WordSpacing
        | HorizontalScale
        | Leading
        | Font
        | TextRenderingMode
        | TextRise
        | TextMove
        | TextMoveLeading
        | TextMatrix
        | TextNextLine
        | ShowText
        | ShowTextArray
        | NextLineShowText
        | SpacedNextLineShowText => Route::Text,
        _ => return None,
    })
}
fn is_path_construction(kind: OperatorKind) -> bool {
    use OperatorKind::*;
    matches!(
        kind,
        MoveTo
            | LineTo
            | CurveTo
            | CurveToV
            | CurveToY
            | ClosePath
            | Rectangle
            | Stroke
            | CloseStroke
            | Fill
            | EvenOddFill
            | FillStroke
            | EvenOddFillStroke
            | CloseFillStroke
            | CloseEvenOddFillStroke
            | EndPath
            | Clip
            | EvenOddClip
    )
}

/// No child prefix or mutable arena is exposed. Every failure is sticky.
/// Single-use LIFO token. PP1 owns one interpreter and must keep its tokens
/// with that interpreter; this crate-private ID is not cross-interpreter authentication.
#[derive(Debug)]
pub(crate) struct FormFrame {
    id: usize,
}
#[derive(Clone, Copy)]
struct SavedFormFrame {
    id: usize,
    floor: usize,
}
const MAX_FORM_DEPTH: usize = 64;
pub struct ProgramInterpreter<'fonts, 'font> {
    paths: PathInterpreter,
    text: TextPositioner<'fonts, 'font>,
    limits: ProgramLimits,
    events: Vec<ProgramEvent>,
    text_contexts: Vec<TextContext>,
    images: Vec<ImagePaint>,
    usage: ProgramUsage,
    save_depth: usize,
    // Fixed structural partition: at most64 small frame descriptors; no input payload.
    form_frames: [Option<SavedFormFrame>; MAX_FORM_DEPTH],
    form_depth: usize,
    inside_text: bool,
    source_index: usize,
    span: Range<usize>,
    failed: Option<ProgramError>,
    /// Per open BMC/BDC: whether it hid its content (hidden optional content).
    marked: Vec<bool>,
    /// Open hiding scopes: hidden marked content, Forms and images.
    hidden: usize,
    /// Visibility the caller resolved for the next BDC /OC operation.
    pending_visibility: Option<bool>,
}
impl<'fonts, 'font> ProgramInterpreter<'fonts, 'font> {
    pub fn new(
        fonts: &'fonts TextFonts<'font>,
        initial: ProgramInitialState,
        limits: ProgramLimits,
    ) -> Result<Self, ProgramError> {
        let error = |kind| ProgramError {
            operation_index: 0,
            span: 0..0,
            kind,
        };
        let paths = PathInterpreter::new(
            PathInitialState::new(initial.ctm, initial.clip),
            limits.path,
        )
        .map_err(|e| error(Failure::Path(Box::new(e))))?;
        let text = TextPositioner::new(
            fonts,
            TextPositionInitialState::new(initial.ctm),
            limits.text,
        )
        .map_err(|e| error(Failure::Text(Box::new(e))))?;
        Ok(Self {
            paths,
            text,
            limits,
            events: Vec::new(),
            text_contexts: Vec::new(),
            images: Vec::new(),
            usage: ProgramUsage::default(),
            save_depth: 0,
            form_frames: [None; MAX_FORM_DEPTH],
            form_depth: 0,
            inside_text: false,
            source_index: 0,
            span: 0..0,
            failed: None,
            marked: Vec::new(),
            hidden: 0,
            pending_visibility: None,
        })
    }
    /// Push BDC /OC with the visibility the caller resolved from resources.
    /// Hidden content still updates graphics and text state but paints nothing.
    pub(crate) fn push_optional_content(
        &mut self,
        operation: &ValidatedOperation<'_>,
        visible: bool,
    ) -> Result<(), ProgramError> {
        self.pending_visibility = Some(visible);
        let result = self.push_with_font(operation, None);
        self.pending_visibility = None;
        result
    }
    /// Push cs/CS with the component count the caller resolved from resources
    /// (1, 3 or 4 for a device-equivalent space). Other spaces stay refused.
    pub(crate) fn push_color_space(
        &mut self,
        operation: &ValidatedOperation<'_>,
        components: usize,
    ) -> Result<(), ProgramError> {
        self.paths.pending_space = Some(components);
        let result = self.push_with_font(operation, None);
        self.paths.pending_space = None;
        result
    }
    /// Hide a whole Form or image whose /OC group is off.
    pub(crate) fn begin_hidden(&mut self) {
        self.hidden += 1;
    }
    pub(crate) fn end_hidden(&mut self) {
        self.hidden -= 1;
    }
    pub fn usage(&self) -> ProgramUsage {
        self.usage
    }
    fn error(&self, kind: Failure) -> ProgramError {
        ProgramError {
            operation_index: self.source_index,
            span: self.span.clone(),
            kind,
        }
    }
    pub fn push(&mut self, operation: &ValidatedOperation<'_>) -> Result<(), ProgramError> {
        self.push_with_font(operation, None)
    }
    pub(crate) fn push_resolved_font(
        &mut self,
        operation: &ValidatedOperation<'_>,
        id: TextFontId,
    ) -> Result<(), ProgramError> {
        self.push_with_font(operation, Some(id))
    }
    fn push_with_font(
        &mut self,
        operation: &ValidatedOperation<'_>,
        resolved_font: Option<TextFontId>,
    ) -> Result<(), ProgramError> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        self.source_index = self.usage.operations;
        self.span = operation.operation().span.clone();
        if let Err(kind) = self.execute(operation, resolved_font) {
            let error = self.error(kind);
            self.failed = Some(error.clone());
            Err(error)
        } else {
            Ok(())
        }
    }
    pub(crate) fn push_image(
        &mut self,
        image_id: usize,
        operation: &Operation,
    ) -> Result<(), ProgramError> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        self.source_index = self.usage.operations;
        self.span = operation.span.clone();
        let result = (|| {
            self.admit()?;
            if self.span.start > self.span.end {
                return Err(Failure::InvalidSpan);
            }
            self.check_invariants()?;
            let validated = validate_operation(operation).map_err(Failure::Operator)?;
            if !matches!(
                validated.kind(),
                OperatorKind::XObject | OperatorKind::InlineImage
            ) {
                return Err(Failure::State("image dispatch requires Do"));
            }
            if self.inside_text || self.paths.snapshot().active {
                return Err(Failure::State(
                    "image Do inside text or active/pending path is unsupported",
                ));
            }
            count(
                self.events.len(),
                1,
                self.limits.max_events,
                ProgramLimitKind::Events,
            )?;
            reserve(
                &mut self.events,
                1,
                self.limits.max_events,
                &mut self.usage,
                self.limits.max_allocated_bytes,
            )?;
            reserve(
                &mut self.images,
                1,
                self.limits.max_events,
                &mut self.usage,
                self.limits.max_allocated_bytes,
            )?;
            if self.hidden > 0 {
                return Ok(());
            }
            let state = self.paths.snapshot();
            let index = self.images.len();
            self.images.push(ImagePaint {
                image_id,
                ctm: state.ctm,
                clip: state.clip,
                operation_index: self.source_index,
            });
            self.events.push(ProgramEvent {
                operation_index: self.source_index,
                span: self.span.clone(),
                kind: ProgramEventKind::Image(index),
            });
            Ok(())
        })();
        result.map_err(|kind| {
            let error = self.error(kind);
            self.failed = Some(error.clone());
            error
        })
    }
    fn boundary<T>(
        &mut self,
        action: impl FnOnce(&mut Self) -> Result<T, Failure>,
    ) -> Result<T, ProgramError> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        self.source_index = self.usage.operations;
        self.span = 0..0;
        let result = self
            .admit()
            .and_then(|()| self.check_invariants())
            .and_then(|()| action(self));
        match result {
            Ok(value) => Ok(value),
            Err(kind) => {
                let error = self.error(kind);
                self.failed = Some(error.clone());
                Err(error)
            }
        }
    }
    fn form_idle(&self) -> Result<(), Failure> {
        if self.inside_text {
            return Err(Failure::State("Form boundary inside BT is unsupported"));
        }
        if self.paths.snapshot().active {
            return Err(Failure::State(
                "Form boundary with active or pending path is unsupported",
            ));
        }
        Ok(())
    }
    /// Typed internal dispatch: no serialization/reparsing. Small temporary
    /// operand/name buffers pay full allocation requests before construction.
    fn form_operation(&mut self, name: &[u8], numbers: &[f64]) -> Result<(), Failure> {
        let count = numbers.len();
        self.charge_work(1 + count + name.len())?;
        let mut operator = Vec::new();
        reserve(
            &mut operator,
            name.len(),
            name.len(),
            &mut self.usage,
            self.limits.max_allocated_bytes,
        )?;
        operator.extend_from_slice(name);
        let mut operands = Vec::new();
        reserve(
            &mut operands,
            count,
            count,
            &mut self.usage,
            self.limits.max_allocated_bytes,
        )?;
        operands.extend(numbers.iter().map(|&v| Value::Real(v)));
        let operation = Operation {
            operator,
            operands,
            span: 0..0,
        };
        let validated = validate_operation(&operation).map_err(Failure::Operator)?;
        self.execute_body(&validated, None)
    }
    pub(crate) fn enter_form(
        &mut self,
        matrix: Affine<UserSpace, UserSpace>,
        bbox: Rect<UserSpace>,
    ) -> Result<FormFrame, ProgramError> {
        self.boundary(|this| {
            if bbox.is_empty() {
                return Err(Failure::State("empty Form BBox is unsupported"));
            }
            matrix
                .inverse()
                .map_err(|_| Failure::State("singular or unrepresentable Form Matrix"))?;
            let frame = this.save_frame()?;
            this.form_operation(b"cm", &matrix.coefficients())?;
            let [x0, y0, x1, y1] = bbox.bounds();
            // Use the original four corners, avoiding x0+(x1-x0) cancellation.
            this.form_operation(b"m", &[x0, y0])?;
            this.form_operation(b"l", &[x1, y0])?;
            this.form_operation(b"l", &[x1, y1])?;
            this.form_operation(b"l", &[x0, y1])?;
            this.form_operation(b"h", &[])?;
            this.form_operation(b"W", &[])?;
            this.form_operation(b"n", &[])?;
            Ok(frame)
        })
    }
    /// Page content runs inside its own q/Q when annotations follow, so their
    /// appearances start from the initial graphics state, not whatever the
    /// content left at depth zero. Closed by `leave_form`.
    pub(crate) fn enter_page_scope(&mut self) -> Result<FormFrame, ProgramError> {
        self.boundary(|this| this.save_frame())
    }
    fn save_frame(&mut self) -> Result<FormFrame, Failure> {
        self.form_idle()?;
        if self.form_depth == MAX_FORM_DEPTH {
            return Err(Failure::State("Form structural depth limit"));
        }
        let id = self.source_index;
        self.form_operation(b"q", &[])?;
        self.form_frames[self.form_depth] = Some(SavedFormFrame {
            id,
            floor: self.save_depth,
        });
        self.form_depth += 1;
        Ok(FormFrame { id })
    }
    pub(crate) fn leave_form(&mut self, frame: FormFrame) -> Result<(), ProgramError> {
        self.boundary(|this| {
            this.form_idle()?;
            let saved = this
                .form_depth
                .checked_sub(1)
                .and_then(|i| this.form_frames[i])
                .ok_or(Failure::State("no active Form frame"))?;
            if saved.id != frame.id {
                return Err(Failure::State("stale or out-of-order Form frame"));
            }
            if this.save_depth != saved.floor {
                return Err(Failure::State("unbalanced Form graphics saves"));
            }
            this.form_depth -= 1;
            this.form_frames[this.form_depth] = None;
            this.form_operation(b"Q", &[])?;
            this.check_invariants()
        })
    }
    pub fn finish(self) -> Result<Program, ProgramError> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        self.check_invariants().map_err(|kind| self.error(kind))?;
        if self.inside_text {
            return Err(self.error(Failure::State("unclosed text object")));
        }
        if self.form_depth != 0 {
            return Err(self.error(Failure::State("unclosed Form frame")));
        }
        if self.save_depth != 0 {
            return Err(self.error(Failure::State("unclosed graphics save")));
        }
        let index = self.source_index;
        let span = self.span;
        let error = |kind| ProgramError {
            operation_index: index,
            span: span.clone(),
            kind,
        };
        let paths = self
            .paths
            .finish()
            .map_err(|e| error(Failure::Path(Box::new(e))))?;
        let text = self
            .text
            .finish()
            .map_err(|e| error(Failure::Text(Box::new(e))))?;
        Ok(Program {
            paths,
            text,
            events: self.events,
            text_contexts: self.text_contexts,
            images: self.images,
            usage: self.usage,
        })
    }
    fn check_invariants(&self) -> Result<(), Failure> {
        let p = self.paths.snapshot();
        if p.ctm.coefficients() != self.text.ctm().coefficients() {
            return Err(Failure::Invariant("path/text CTMs diverged"));
        }
        if p.save_depth != self.save_depth || self.text.save_depth() != self.save_depth {
            return Err(Failure::Invariant("graphics stack depths diverged"));
        }
        if self.text.inside_text() != self.inside_text {
            return Err(Failure::Invariant("text-object state diverged"));
        }
        Ok(())
    }
    fn charge_child(&mut self, before: usize, after: usize) -> Result<(), Failure> {
        let delta = after
            .checked_sub(before)
            .ok_or(Failure::Invariant("child allocation counter decreased"))?;
        self.usage.allocated_bytes = self
            .usage
            .allocated_bytes
            .checked_add(delta)
            .filter(|&n| n <= self.limits.max_allocated_bytes)
            .ok_or(Failure::Invariant(
                "child exceeded pre-admitted allocation allowance",
            ))?;
        Ok(())
    }
    fn push_path(&mut self, operation: &ValidatedOperation<'_>) -> Result<(), Failure> {
        let before = self.paths.allocated_bytes();
        self.usage.child_operations = self
            .usage
            .child_operations
            .checked_add(1)
            .ok_or_else(|| exceed(ProgramLimitKind::WorkUnits))?;
        let result = self.paths.push_budgeted(
            operation,
            self.limits.max_allocated_bytes - self.usage.allocated_bytes,
        );
        self.charge_child(before, self.paths.allocated_bytes())?;
        result.map_err(|e| Failure::Path(Box::new(e)))
    }
    fn push_text(
        &mut self,
        operation: &ValidatedOperation<'_>,
        resolved_font: Option<TextFontId>,
    ) -> Result<(), Failure> {
        let before = self.text.allocated_bytes();
        self.usage.child_operations = self
            .usage
            .child_operations
            .checked_add(1)
            .ok_or_else(|| exceed(ProgramLimitKind::WorkUnits))?;
        let allowance = self.limits.max_allocated_bytes - self.usage.allocated_bytes;
        let result = if let Some(id) = resolved_font {
            self.text
                .push_resolved_budgeted(operation, self.source_index, allowance, id)
        } else {
            self.text
                .push_budgeted(operation, self.source_index, allowance)
        };
        self.charge_child(before, self.text.allocated_bytes())?;
        result.map_err(|e| Failure::Text(Box::new(e)))
    }
    fn charge_work(&mut self, amount: usize) -> Result<(), Failure> {
        self.usage.work_units = count(
            self.usage.work_units,
            amount,
            self.limits.max_work_units,
            ProgramLimitKind::WorkUnits,
        )?;
        Ok(())
    }
    /// Exact output count for one-byte text fonts; path state supplies the exact
    /// paint/clip event count. Bounds precede decoding and coordinator allocation.
    fn preflight(
        &mut self,
        operation: &ValidatedOperation<'_>,
        route: Route,
    ) -> Result<usize, Failure> {
        use OperatorKind::*;
        self.charge_work(if matches!(route, Route::Shared) { 2 } else { 1 })?;
        let mut events = if matches!(route, Route::Path | Route::Shared) {
            self.paths.event_upper_bound(operation.kind())
        } else {
            0
        };
        match operation.kind() {
            ShowText | NextLineShowText | SpacedNextLineShowText => {
                let operand = if operation.kind() == SpacedNextLineShowText {
                    2
                } else {
                    0
                };
                let Value::String(bytes) = &operation.operands()[operand] else {
                    unreachable!()
                };
                self.charge_work(bytes.len())?;
                events = bytes.len();
            }
            ShowTextArray => {
                let Value::Array(items) = &operation.operands()[0] else {
                    unreachable!()
                };
                self.charge_work(items.len())?;
                for item in items {
                    match item {
                        Value::String(bytes) => {
                            self.charge_work(bytes.len())?;
                            events = count(
                                events,
                                bytes.len(),
                                self.limits.max_events,
                                ProgramLimitKind::Events,
                            )?;
                        }
                        _ => {
                            events =
                                count(events, 1, self.limits.max_events, ProgramLimitKind::Events)?
                        }
                    }
                }
            }
            EndText => events = 1,
            Dash => {
                let Value::Array(items) = &operation.operands()[0] else {
                    unreachable!()
                };
                self.charge_work(items.len())?;
            }
            _ => {}
        }
        count(
            self.events.len(),
            events,
            self.limits.max_events,
            ProgramLimitKind::Events,
        )?;
        Ok(events)
    }
    fn execute(
        &mut self,
        operation: &ValidatedOperation<'_>,
        resolved_font: Option<TextFontId>,
    ) -> Result<(), Failure> {
        self.admit()?;
        self.execute_body(operation, resolved_font)
    }
    fn admit(&mut self) -> Result<(), Failure> {
        self.usage.operations = count(
            self.usage.operations,
            1,
            self.limits.max_operations,
            ProgramLimitKind::Operations,
        )?;
        self.usage.work_units = count(
            self.usage.work_units,
            1,
            self.limits.max_work_units,
            ProgramLimitKind::WorkUnits,
        )?;
        Ok(())
    }
    fn execute_body(
        &mut self,
        operation: &ValidatedOperation<'_>,
        resolved_font: Option<TextFontId>,
    ) -> Result<(), Failure> {
        if self.span.start > self.span.end {
            return Err(Failure::InvalidSpan);
        }
        self.check_invariants()?;
        let kind = operation.kind();
        if resolved_font.is_some() {
            if kind != OperatorKind::Font {
                return Err(Failure::State(
                    "resolved font supplied for non-Tf operation",
                ));
            }
            let Value::Name(name) = &operation.operands()[0] else {
                unreachable!()
            };
            self.charge_work(name.len())?;
            // T0 checks whether this selected ID already has an owned record.
            // Charge the full possible scan before dispatch, including repeats.
            self.charge_work(self.text.selected_font_count())?;
        }
        if kind == OperatorKind::RenderingIntent {
            // The bounded device-color preview starts in RelativeColorimetric
            // and admits no operation that changes that intent. Therefore q/Q
            // and Form saves always restore the same value. Calibrated/ICC and
            // resource default color spaces remain outside this pipeline.
            if relative_colorimetric(operation) {
                self.charge_work(b"RelativeColorimetric".len())?;
                return Ok(());
            }
            return Err(Failure::Unsupported(kind));
        }
        if matches!(
            kind,
            OperatorKind::MarkedPoint
                | OperatorKind::MarkedPointProperties
                | OperatorKind::BeginMarkedContent
                | OperatorKind::BeginMarkedContentProperties
                | OperatorKind::EndMarkedContent
        ) {
            // Page preflight checks per-stream BMC/EMC balance. Marked content
            // has no rendering or text-position effect, except optional
            // content (/OC), which the caller must resolve.
            match kind {
                OperatorKind::BeginMarkedContent | OperatorKind::BeginMarkedContentProperties => {
                    let hide = if optional_content(operation) {
                        !self.pending_visibility.ok_or(Failure::Unsupported(kind))?
                    } else {
                        false
                    };
                    if self.marked.len() >= self.limits.max_save_depth {
                        return Err(Failure::State("marked-content depth"));
                    }
                    self.marked.push(hide);
                    self.hidden += usize::from(hide);
                }
                OperatorKind::EndMarkedContent if self.marked.pop() == Some(true) => {
                    self.hidden -= 1;
                }
                _ => {}
            }
            return Ok(());
        }
        if kind == OperatorKind::Flatness {
            return if default_flatness(operation) {
                Ok(())
            } else {
                Err(Failure::Unsupported(kind))
            };
        }
        let route = route(kind).ok_or(Failure::Unsupported(kind))?;
        use OperatorKind::*;
        if matches!(route, Route::Shared) && self.inside_text {
            return Err(Failure::State("q/Q/cm inside BT is unsupported"));
        }
        if is_path_construction(kind) && self.inside_text {
            return Err(Failure::State(
                "path construction or painting inside BT is unsupported",
            ));
        }
        match kind {
            Save => {
                count(
                    self.save_depth,
                    1,
                    self.limits.max_save_depth,
                    ProgramLimitKind::SaveDepth,
                )?;
            }
            Restore if self.save_depth == 0 => {
                return Err(Failure::State("graphics stack underflow"));
            }
            Restore
                if self.form_depth > 0
                    && self.save_depth
                        <= self.form_frames[self.form_depth - 1]
                            .ok_or(Failure::Invariant("missing Form floor"))?
                            .floor =>
            {
                return Err(Failure::State("Q crosses implicit Form save"));
            }
            BeginText => {
                if self.inside_text {
                    return Err(Failure::State("nested BT"));
                }
                if self.paths.snapshot().active {
                    return Err(Failure::State(
                        "BT with active or pending path is unsupported",
                    ));
                }
            }
            EndText if !self.inside_text => return Err(Failure::State("ET outside BT")),
            _ => {}
        }
        let event_upper = self.preflight(operation, route)?;
        let p = self.paths.snapshot();
        let context = TextContext {
            ctm: p.ctm,
            clip: p.clip,
            fill_color: p.fill,
        };
        let context_needed = matches!(route, Route::Text) && event_upper != 0;
        let reuse = context_needed && self.text_contexts.last() == Some(&context);
        if context_needed && !reuse {
            count(
                self.text_contexts.len(),
                1,
                self.limits.max_text_contexts,
                ProgramLimitKind::TextContexts,
            )?;
        }
        let path_start = self.paths.event_count();
        let text_start = self.text.event_count();
        // P0 first, then T0. A second-child failure never publishes the private
        // P0 mutation; its allocation/work remains charged and the parent sticks.
        match route {
            Route::Shared => {
                self.push_path(operation)?;
                self.push_text(operation, resolved_font)?;
            }
            Route::Path => self.push_path(operation)?,
            Route::Text => self.push_text(operation, resolved_font)?,
        }
        match kind {
            Save => self.save_depth += 1,
            Restore => self.save_depth -= 1,
            BeginText => self.inside_text = true,
            EndText => self.inside_text = false,
            _ => {}
        }
        self.check_invariants()?;
        let path_end = self.paths.event_count();
        let text_end = self.text.event_count();
        let new_events = (path_end - path_start)
            .checked_add(text_end - text_start)
            .ok_or(Failure::Invariant("event count overflow"))?;
        if new_events > event_upper {
            return Err(Failure::Invariant("event preflight underestimated output"));
        }
        let context_id = if text_end > text_start {
            if reuse {
                self.text_contexts.len() - 1
            } else {
                reserve(
                    &mut self.text_contexts,
                    1,
                    self.limits.max_text_contexts,
                    &mut self.usage,
                    self.limits.max_allocated_bytes,
                )?;
                let id = self.text_contexts.len();
                self.text_contexts.push(context);
                id
            }
        } else {
            0
        };
        reserve(
            &mut self.events,
            new_events,
            self.limits.max_events,
            &mut self.usage,
            self.limits.max_allocated_bytes,
        )?;
        for event in path_start..path_end {
            if self.hidden > 0 && self.paths.paints(event) {
                continue;
            }
            self.events.push(ProgramEvent {
                operation_index: self.source_index,
                span: self.span.clone(),
                kind: ProgramEventKind::Path(event),
            });
        }
        for event in text_start..text_end {
            if self.hidden > 0 && self.text.paints(event) {
                continue;
            }
            self.events.push(ProgramEvent {
                operation_index: self.source_index,
                span: self.span.clone(),
                kind: ProgramEventKind::Text {
                    event,
                    context: context_id,
                },
            });
        }
        Ok(())
    }
}
/// Parse and validate under separate ContentLimits, then execute atomically.
/// Parent execution work is not a substitute for parser/input/font limits.
pub fn interpret_program_content(
    input: &[u8],
    fonts: &TextFonts<'_>,
    initial: ProgramInitialState,
    content_limits: ContentLimits,
    limits: ProgramLimits,
) -> Result<Program, ProgramError> {
    let mut interpreter = ProgramInterpreter::new(fonts, initial, limits)?;
    let mut parser = ContentParser::new(input, content_limits);
    loop {
        let operation = parser.next_operation().map_err(|e| ProgramError {
            operation_index: interpreter.usage.operations,
            span: e.offset..e.offset,
            kind: Failure::Content(Box::new(e)),
        })?;
        let Some(operation) = operation else {
            break;
        };
        // Before general operator validation walks a public array, bound that
        // traversal by the remaining aggregate execution budget.
        if matches!(operation.operator.as_slice(), b"TJ" | b"d") {
            if let Some(Value::Array(items)) = operation.operands.first() {
                if items.len()
                    > limits
                        .max_work_units
                        .saturating_sub(interpreter.usage.work_units)
                {
                    return Err(ProgramError {
                        operation_index: interpreter.usage.operations,
                        span: operation.span,
                        kind: exceed(ProgramLimitKind::WorkUnits),
                    });
                }
            }
        }
        let validated = validate_operation(&operation).map_err(|e| ProgramError {
            operation_index: interpreter.usage.operations,
            span: operation.span.clone(),
            kind: Failure::Operator(e),
        })?;
        interpreter.push(&validated)?;
    }
    interpreter.finish()
}

#[cfg(test)]
#[path = "program_form_tests.rs"]
mod form_tests;

#[cfg(test)]
#[path = "program_image_tests.rs"]
mod image_tests;
