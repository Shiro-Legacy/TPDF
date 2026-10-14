// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded standalone horizontal text positioning. Records source codes, not
//! glyph outlines or reading order. Source ranges index decoded string bytes.
//!
//! Behavior references MuPDF 1.28.1 pdf-op-run.c and pdf-interpret.c,
//! Copyright (C) 2004–2026 Artifex Software, Inc.; see NOTICE. Unicode and
//! advance policies remain independent, with no recovery or font substitution.
use crate::content::{ContentError, ContentLimits, ContentOperand as Value, ContentParser};
use crate::geometry::{Affine, PageSpace, Point, UserSpace};
use crate::operators::{OperatorError, OperatorKind, ValidatedOperation, validate_operation};
use crate::text_decode::{
    DecodedCode, TextDecodeError, TextDecodeErrorKind, TextDecodeLimitKind, TextDecodeLimits,
};
use crate::text_font::{
    TextFontError, TextFontId, TextFonts, TextMetricKind, TextMetricOrigin, TextUnicodeKind,
};
use std::{error::Error, fmt, mem::size_of, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextSpace {}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EmSpace {}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextPositionInitialState {
    ctm: Affine<UserSpace, PageSpace>,
}
impl TextPositionInitialState {
    pub fn new(ctm: Affine<UserSpace, PageSpace>) -> Self {
        Self { ctm }
    }
    pub fn ctm(self) -> Affine<UserSpace, PageSpace> {
        self.ctm
    }
}
impl Default for TextPositionInitialState {
    fn default() -> Self {
        Self {
            ctm: Affine::from_pdf([1., 0., 0., 1., 0., 0.]).expect("identity is finite"),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextPositionLimits {
    pub max_operations: usize,
    pub max_stack_depth: usize,
    pub max_text_objects: usize,
    pub max_codes: usize,
    pub max_tj_elements: usize,
    pub max_show_operations: usize,
    pub max_source_bytes: usize,
    pub max_scalars: usize,
    pub max_utf8_bytes: usize,
    /// Cumulative requested allocation capacity, including temporary decoding.
    pub max_allocation_bytes: usize,
    pub max_fonts: usize,
    pub max_font_name_bytes: usize,
    pub max_total_font_name_bytes: usize,
    pub max_linear: f64,
    pub max_position: f64,
    pub max_text_value: f64,
}
impl Default for TextPositionLimits {
    fn default() -> Self {
        Self {
            max_operations: 100_000,
            max_stack_depth: 128,
            max_text_objects: 10_000,
            max_codes: 262_144,
            max_tj_elements: 262_144,
            max_show_operations: 262_144,
            max_source_bytes: 1024 * 1024,
            max_scalars: 1_000_000,
            max_utf8_bytes: 4 * 1024 * 1024,
            max_allocation_bytes: 64 * 1024 * 1024,
            max_fonts: 256,
            max_font_name_bytes: 127,
            max_total_font_name_bytes: 32 * 1024,
            max_linear: 1e6,
            max_position: 1e9,
            max_text_value: 1e6,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextPositionLimitKind {
    Operations,
    StackDepth,
    TextObjects,
    Codes,
    TjElements,
    ShowOperations,
    SourceBytes,
    Scalars,
    Utf8Bytes,
    AllocationBytes,
    Fonts,
    FontNameBytes,
    TotalFontNameBytes,
    LinearMagnitude,
    PositionMagnitude,
    TextMagnitude,
}
#[derive(Clone, Debug, PartialEq)]
pub enum TextPositionErrorKind {
    Content(Box<ContentError>),
    Operator(OperatorError),
    Decode(TextDecodeError),
    Font(TextFontError),
    State(&'static str),
    Unsupported(&'static str),
    MissingFont,
    InexactInteger,
    Nonfinite,
    InvalidLimits,
    InvalidSpan,
    LimitExceeded(TextPositionLimitKind),
    AllocationFailed,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TextPositionError {
    pub operation_index: usize,
    pub operation_span: Range<usize>,
    pub operand_index: Option<usize>,
    pub array_index: Option<usize>,
    pub string_offset: Option<usize>,
    pub kind: TextPositionErrorKind,
}
impl fmt::Display for TextPositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "text positioning operation {} at {:?}: {:?}",
            self.operation_index, self.operation_span, self.kind
        )
    }
}
impl Error for TextPositionError {}
type Failure = TextPositionErrorKind;
type ResultKind<T> = Result<T, Failure>;
fn limit(kind: TextPositionLimitKind) -> Failure {
    Failure::LimitExceeded(kind)
}
fn add_count(
    current: usize,
    add: usize,
    maximum: usize,
    kind: TextPositionLimitKind,
) -> ResultKind<usize> {
    current
        .checked_add(add)
        .filter(|&n| n <= maximum)
        .ok_or_else(|| limit(kind))
}
fn finite(value: f64) -> ResultKind<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(Failure::Nonfinite)
    }
}
fn bounded(value: f64, maximum: f64, kind: TextPositionLimitKind) -> ResultKind<f64> {
    finite(value)?;
    if value.abs() > maximum {
        Err(limit(kind))
    } else {
        Ok(value)
    }
}
fn number(value: &Value) -> ResultKind<f64> {
    match value {
        Value::Integer(n) if !(-9_007_199_254_740_992..=9_007_199_254_740_992).contains(n) => {
            Err(Failure::InexactInteger)
        }
        Value::Integer(n) => Ok(*n as f64),
        Value::Real(n) => finite(*n),
        _ => Err(Failure::State("expected validated number")),
    }
}
fn matrix<F, T>(m: Affine<F, T>, limits: TextPositionLimits) -> ResultKind<Affine<F, T>> {
    let c = m.coefficients();
    for v in &c[..4] {
        bounded(
            *v,
            limits.max_linear,
            TextPositionLimitKind::LinearMagnitude,
        )?;
    }
    for v in &c[4..] {
        bounded(
            *v,
            limits.max_position,
            TextPositionLimitKind::PositionMagnitude,
        )?;
    }
    Ok(m)
}
/// Explicit intermediates prohibit finite overflow being hidden by cancellation.
fn compose<F, M, T>(
    first: Affine<F, M>,
    second: Affine<M, T>,
    limits: TextPositionLimits,
) -> ResultKind<Affine<F, T>> {
    let [a, b, c, d, e, f] = first.coefficients();
    let [g, h, i, j, k, l] = second.coefficients();
    let pair = |a: f64, b: f64, c: f64, d: f64| finite(finite(a * b)? + finite(c * d)?);
    let result = [
        pair(a, g, b, i)?,
        pair(a, h, b, j)?,
        pair(c, g, d, i)?,
        pair(c, h, d, j)?,
        finite(pair(e, g, f, i)? + k)?,
        finite(pair(e, h, f, j)? + l)?,
    ];
    matrix(
        Affine::from_pdf(result).map_err(|_| Failure::Nonfinite)?,
        limits,
    )
}
fn reserve<T>(
    vector: &mut Vec<T>,
    additional: usize,
    maximum: usize,
    allocation: &mut usize,
    limit_bytes: usize,
) -> ResultKind<()> {
    let required = vector
        .len()
        .checked_add(additional)
        .ok_or_else(|| limit(TextPositionLimitKind::AllocationBytes))?;
    if required > maximum {
        return Err(limit(TextPositionLimitKind::AllocationBytes));
    }
    if required > vector.capacity() {
        let target = vector
            .capacity()
            .saturating_mul(2)
            .min(maximum)
            .max(required);
        let bytes = target
            .checked_mul(size_of::<T>())
            .ok_or_else(|| limit(TextPositionLimitKind::AllocationBytes))?;
        *allocation = add_count(
            *allocation,
            bytes,
            limit_bytes,
            TextPositionLimitKind::AllocationBytes,
        )?;
        vector
            .try_reserve_exact(target - vector.len())
            .map_err(|_| Failure::AllocationFailed)?;
    }
    Ok(())
}
fn reserve_string(
    value: &mut String,
    additional: usize,
    maximum: usize,
    allocation: &mut usize,
    limit_bytes: usize,
) -> ResultKind<()> {
    let required = value
        .len()
        .checked_add(additional)
        .ok_or_else(|| limit(TextPositionLimitKind::AllocationBytes))?;
    if required > maximum {
        return Err(limit(TextPositionLimitKind::Utf8Bytes));
    }
    if required > value.capacity() {
        let target = value
            .capacity()
            .saturating_mul(2)
            .min(maximum)
            .max(required);
        *allocation = add_count(
            *allocation,
            target,
            limit_bytes,
            TextPositionLimitKind::AllocationBytes,
        )?;
        value
            .try_reserve_exact(target - value.len())
            .map_err(|_| Failure::AllocationFailed)?;
    }
    Ok(())
}
macro_rules! copy_getters { ($($name:ident:$ty:ty),* $(,)?) => {$ (pub fn $name(&self)->$ty {self.$name})*}; }
macro_rules! range_getters { ($($name:ident),* $(,)?) => {$ (pub fn $name(&self)->Range<usize> {self.$name.clone()})*}; }
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedFont {
    id: TextFontId,
    name: Vec<u8>,
    unicode_kind: TextUnicodeKind,
    metric_kind: TextMetricKind,
}
impl PositionedFont {
    copy_getters!(id:TextFontId,unicode_kind:TextUnicodeKind,metric_kind:TextMetricKind);
    pub fn name(&self) -> &[u8] {
        &self.name
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct CodePlacement {
    /// Whole character code: a byte for simple fonts, the CID for Identity-H.
    code: u32,
    font_id: TextFontId,
    text_object: usize,
    operation_index: usize,
    operation_span: Range<usize>,
    operand_index: usize,
    array_index: Option<usize>,
    string_span: Range<usize>,
    source_span: Range<usize>,
    utf8_span: Range<usize>,
    width: f64,
    metric_origin: TextMetricOrigin,
    character_spacing: f64,
    word_spacing: f64,
    font_size: f64,
    horizontal_scale: f64,
    rise: f64,
    rendering_mode: u8,
    before: Affine<TextSpace, UserSpace>,
    after: Affine<TextSpace, UserSpace>,
    transform: Affine<EmSpace, PageSpace>,
    origin: Point<PageSpace>,
    glyph_displacement: f64,
    word_displacement: f64,
}
impl CodePlacement {
    copy_getters!(code:u32,font_id:TextFontId,text_object:usize,operation_index:usize,operand_index:usize,array_index:Option<usize>,width:f64,metric_origin:TextMetricOrigin,
        character_spacing:f64,word_spacing:f64,font_size:f64,horizontal_scale:f64,rise:f64,rendering_mode:u8,
        before:Affine<TextSpace,UserSpace>,after:Affine<TextSpace,UserSpace>,transform:Affine<EmSpace,PageSpace>,origin:Point<PageSpace>,glyph_displacement:f64,word_displacement:f64);
    range_getters!(operation_span, string_span, source_span, utf8_span);
}
#[derive(Clone, Debug, PartialEq)]
pub struct TextAdjustment {
    number: f64,
    displacement: f64,
    before: Affine<TextSpace, UserSpace>,
    after: Affine<TextSpace, UserSpace>,
    operation_index: usize,
    operation_span: Range<usize>,
    array_index: usize,
    text_object: usize,
}
impl TextAdjustment {
    copy_getters!(number:f64,displacement:f64,before:Affine<TextSpace,UserSpace>,after:Affine<TextSpace,UserSpace>,operation_index:usize,array_index:usize,text_object:usize);
    range_getters!(operation_span);
}
#[derive(Clone, Debug, PartialEq)]
pub struct TextObjectEnd {
    text_matrix: Affine<TextSpace, UserSpace>,
    line_matrix: Affine<TextSpace, UserSpace>,
    operation_index: usize,
    operation_span: Range<usize>,
}
impl TextObjectEnd {
    copy_getters!(text_matrix:Affine<TextSpace,UserSpace>,line_matrix:Affine<TextSpace,UserSpace>,operation_index:usize);
    range_getters!(operation_span);
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextEvent {
    Code(usize),
    Adjustment(usize),
    EndTextObject(usize),
}
#[derive(Clone, Debug, PartialEq)]
pub struct PositionedText {
    text: String,
    source_bytes: Vec<u8>,
    fonts: Vec<PositionedFont>,
    codes: Vec<CodePlacement>,
    adjustments: Vec<TextAdjustment>,
    text_objects: Vec<TextObjectEnd>,
    events: Vec<TextEvent>,
}
impl PositionedText {
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn source_bytes(&self) -> &[u8] {
        &self.source_bytes
    }
    pub fn fonts(&self) -> &[PositionedFont] {
        &self.fonts
    }
    pub fn codes(&self) -> &[CodePlacement] {
        &self.codes
    }
    pub fn adjustments(&self) -> &[TextAdjustment] {
        &self.adjustments
    }
    pub fn text_objects(&self) -> &[TextObjectEnd] {
        &self.text_objects
    }
    pub fn events(&self) -> &[TextEvent] {
        &self.events
    }
}
#[derive(Clone, Copy, Debug)]
struct GraphicsState {
    ctm: Affine<UserSpace, PageSpace>,
    font: Option<TextFontId>,
    size: f64,
    tc: f64,
    tw: f64,
    h: f64,
    leading: f64,
    rise: f64,
    tr: u8,
}
#[derive(Clone, Copy, Debug)]
struct TextObject {
    tm: Affine<TextSpace, UserSpace>,
    tlm: Affine<TextSpace, UserSpace>,
}

/// No partial output is exposed. An error permanently invalidates this executor.
pub struct TextPositioner<'fonts, 'font> {
    fonts: &'fonts TextFonts<'font>,
    limits: TextPositionLimits,
    state: GraphicsState,
    stack: Vec<GraphicsState>,
    object: Option<TextObject>,
    result: PositionedText,
    operations: usize,
    source_index: usize,
    objects: usize,
    tj_elements: usize,
    shows: usize,
    scalars: usize,
    allocation: usize,
    name_bytes: usize,
    operation_span: Range<usize>,
    operand_index: Option<usize>,
    array_index: Option<usize>,
    string_offset: Option<usize>,
    failed: Option<TextPositionError>,
}
impl<'fonts, 'font> TextPositioner<'fonts, 'font> {
    pub fn new(
        fonts: &'fonts TextFonts<'font>,
        initial: TextPositionInitialState,
        limits: TextPositionLimits,
    ) -> Result<Self, TextPositionError> {
        let failure = |kind| TextPositionError {
            operation_index: 0,
            operation_span: 0..0,
            operand_index: None,
            array_index: None,
            string_offset: None,
            kind,
        };
        if [
            limits.max_linear,
            limits.max_position,
            limits.max_text_value,
        ]
        .iter()
        .any(|n| !n.is_finite() || *n < 0.)
        {
            return Err(failure(Failure::InvalidLimits));
        }
        matrix(initial.ctm, limits).map_err(failure)?;
        Ok(Self {
            fonts,
            limits,
            state: GraphicsState {
                ctm: initial.ctm,
                font: None,
                size: 0.,
                tc: 0.,
                tw: 0.,
                h: 1.,
                leading: 0.,
                rise: 0.,
                tr: 0,
            },
            stack: Vec::new(),
            object: None,
            result: PositionedText {
                text: String::new(),
                source_bytes: Vec::new(),
                fonts: Vec::new(),
                codes: Vec::new(),
                adjustments: Vec::new(),
                text_objects: Vec::new(),
                events: Vec::new(),
            },
            operations: 0,
            source_index: 0,
            objects: 0,
            tj_elements: 0,
            shows: 0,
            scalars: 0,
            allocation: 0,
            name_bytes: 0,
            operation_span: 0..0,
            operand_index: None,
            array_index: None,
            string_offset: None,
            failed: None,
        })
    }
    pub(crate) fn ctm(&self) -> Affine<UserSpace, PageSpace> {
        self.state.ctm
    }
    pub(crate) fn save_depth(&self) -> usize {
        self.stack.len()
    }
    pub(crate) fn inside_text(&self) -> bool {
        self.object.is_some()
    }
    pub(crate) fn selected_font_count(&self) -> usize {
        self.result.fonts.len()
    }
    pub(crate) fn event_count(&self) -> usize {
        self.result.events.len()
    }
    /// A placed code paints unless its rendering mode adds to the clip.
    /// ponytail: modes 4-6 both paint and clip; they stay visible when hidden.
    pub(crate) fn paints(&self, event: usize) -> bool {
        matches!(self.result.events[event], TextEvent::Code(code) if self.result.codes[code].rendering_mode() < 4)
    }
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.allocation
    }
    pub(crate) fn push_budgeted(
        &mut self,
        operation: &ValidatedOperation<'_>,
        source_index: usize,
        allowance: usize,
    ) -> Result<(), TextPositionError> {
        let configured = self.limits.max_allocation_bytes;
        self.limits.max_allocation_bytes =
            configured.min(self.allocation.saturating_add(allowance));
        let result = self.push_indexed(operation, source_index, None);
        self.limits.max_allocation_bytes = configured;
        result
    }
    pub(crate) fn push_resolved_budgeted(
        &mut self,
        operation: &ValidatedOperation<'_>,
        source_index: usize,
        allowance: usize,
        font: TextFontId,
    ) -> Result<(), TextPositionError> {
        let configured = self.limits.max_allocation_bytes;
        self.limits.max_allocation_bytes =
            configured.min(self.allocation.saturating_add(allowance));
        let result = self.push_indexed(operation, source_index, Some(font));
        self.limits.max_allocation_bytes = configured;
        result
    }
    fn error(&self, kind: Failure) -> TextPositionError {
        TextPositionError {
            operation_index: self.source_index,
            operation_span: self.operation_span.clone(),
            operand_index: self.operand_index,
            array_index: self.array_index,
            string_offset: self.string_offset,
            kind,
        }
    }
    pub fn push(&mut self, operation: &ValidatedOperation<'_>) -> Result<(), TextPositionError> {
        self.push_indexed(operation, self.operations, None)
    }
    fn push_indexed(
        &mut self,
        operation: &ValidatedOperation<'_>,
        source_index: usize,
        resolved_font: Option<TextFontId>,
    ) -> Result<(), TextPositionError> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        self.source_index = source_index;
        self.operation_span = operation.operation().span.clone();
        self.operand_index = None;
        self.array_index = None;
        self.string_offset = None;
        let attempted_index = source_index;
        let result = self.execute(operation, resolved_font);
        if let Err(kind) = result {
            let mut error = self.error(kind);
            error.operation_index = attempted_index;
            self.failed = Some(error.clone());
            Err(error)
        } else {
            Ok(())
        }
    }
    pub fn finish(self) -> Result<PositionedText, TextPositionError> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        if self.object.is_some() {
            return Err(self.error(Failure::State("unclosed text object")));
        }
        if !self.stack.is_empty() {
            return Err(self.error(Failure::State("unclosed graphics save")));
        }
        Ok(self.result)
    }
    fn event(&mut self, event: TextEvent) -> ResultKind<()> {
        let max = self
            .limits
            .max_codes
            .saturating_add(self.limits.max_tj_elements)
            .saturating_add(self.limits.max_text_objects);
        reserve(
            &mut self.result.events,
            1,
            max,
            &mut self.allocation,
            self.limits.max_allocation_bytes,
        )?;
        self.result.events.push(event);
        Ok(())
    }
    fn text_value(&self, n: f64) -> ResultKind<f64> {
        bounded(
            n,
            self.limits.max_text_value,
            TextPositionLimitKind::TextMagnitude,
        )
    }
    fn position_value(&self, n: f64) -> ResultKind<f64> {
        bounded(
            n,
            self.limits.max_position,
            TextPositionLimitKind::PositionMagnitude,
        )
    }
    fn text_object(&self) -> ResultKind<TextObject> {
        self.object
            .ok_or(Failure::State("text operator outside BT"))
    }
    fn outside(&self) -> ResultKind<()> {
        if self.object.is_some() {
            Err(Failure::Unsupported("q/Q/cm inside BT"))
        } else {
            Ok(())
        }
    }
    fn translate(
        &self,
        tm: Affine<TextSpace, UserSpace>,
        x: f64,
        y: f64,
    ) -> ResultKind<Affine<TextSpace, UserSpace>> {
        self.position_value(x)?;
        self.position_value(y)?;
        compose(
            Affine::<TextSpace, TextSpace>::translation(x, y).map_err(|_| Failure::Nonfinite)?,
            tm,
            self.limits,
        )
    }
    fn move_line(&mut self, x: f64, y: f64) -> ResultKind<()> {
        let old = self.text_object()?;
        let tm = self.translate(old.tlm, x, y)?;
        self.object = Some(TextObject { tm, tlm: tm });
        Ok(())
    }
    fn select_font(
        &mut self,
        name: &[u8],
        size: f64,
        resolved: Option<TextFontId>,
    ) -> ResultKind<()> {
        if name.len() > self.limits.max_font_name_bytes {
            return Err(limit(TextPositionLimitKind::FontNameBytes));
        }
        let (id, font) = if let Some(id) = resolved {
            if self.fonts.name(id) != Some(name) {
                return Err(Failure::MissingFont);
            }
            (id, self.fonts.by_id(id).ok_or(Failure::MissingFont)?)
        } else {
            self.fonts.get(name).ok_or(Failure::MissingFont)?
        };
        if !self.result.fonts.iter().any(|f| f.id == id) {
            add_count(
                self.result.fonts.len(),
                1,
                self.limits.max_fonts,
                TextPositionLimitKind::Fonts,
            )?;
            self.name_bytes = add_count(
                self.name_bytes,
                name.len(),
                self.limits.max_total_font_name_bytes,
                TextPositionLimitKind::TotalFontNameBytes,
            )?;
            let mut owned = Vec::new();
            reserve(
                &mut owned,
                name.len(),
                name.len(),
                &mut self.allocation,
                self.limits.max_allocation_bytes,
            )?;
            owned.extend_from_slice(name);
            reserve(
                &mut self.result.fonts,
                1,
                self.limits.max_fonts,
                &mut self.allocation,
                self.limits.max_allocation_bytes,
            )?;
            self.result.fonts.push(PositionedFont {
                id,
                name: owned,
                unicode_kind: font.unicode_kind(),
                metric_kind: font.metric_kind(),
            });
        }
        self.state.font = Some(id);
        self.state.size = size;
        Ok(())
    }
    fn execute(
        &mut self,
        operation: &ValidatedOperation<'_>,
        resolved_font: Option<TextFontId>,
    ) -> ResultKind<()> {
        self.operations = add_count(
            self.operations,
            1,
            self.limits.max_operations,
            TextPositionLimitKind::Operations,
        )?;
        if self.operation_span.start > self.operation_span.end {
            return Err(Failure::InvalidSpan);
        }
        if resolved_font.is_some() && operation.kind() != OperatorKind::Font {
            return Err(Failure::State(
                "resolved font supplied for non-Tf operation",
            ));
        }
        let v = operation.operands();
        use OperatorKind::*;
        match operation.kind() {
            Save => {
                self.outside()?;
                add_count(
                    self.stack.len(),
                    1,
                    self.limits.max_stack_depth,
                    TextPositionLimitKind::StackDepth,
                )?;
                reserve(
                    &mut self.stack,
                    1,
                    self.limits.max_stack_depth,
                    &mut self.allocation,
                    self.limits.max_allocation_bytes,
                )?;
                self.stack.push(self.state);
            }
            Restore => {
                self.outside()?;
                self.state = self
                    .stack
                    .pop()
                    .ok_or(Failure::State("graphics stack underflow"))?;
            }
            Concat | TextMatrix => {
                if operation.kind() == Concat {
                    self.outside()?;
                } else {
                    self.text_object()?;
                }
                let mut c = [0.; 6];
                for (i, n) in v.iter().enumerate() {
                    self.operand_index = Some(i);
                    c[i] = number(n)?;
                }
                if operation.kind() == Concat {
                    let m = matrix(
                        Affine::from_pdf(c).map_err(|_| Failure::Nonfinite)?,
                        self.limits,
                    )?;
                    self.state.ctm = compose(m, self.state.ctm, self.limits)?;
                } else {
                    let m = matrix(
                        Affine::from_pdf(c).map_err(|_| Failure::Nonfinite)?,
                        self.limits,
                    )?;
                    self.object = Some(TextObject { tm: m, tlm: m });
                }
            }
            BeginText => {
                if self.object.is_some() {
                    return Err(Failure::State("nested BT"));
                }
                self.objects = add_count(
                    self.objects,
                    1,
                    self.limits.max_text_objects,
                    TextPositionLimitKind::TextObjects,
                )?;
                let tm = matrix(
                    Affine::from_pdf([1., 0., 0., 1., 0., 0.]).map_err(|_| Failure::Nonfinite)?,
                    self.limits,
                )?;
                self.object = Some(TextObject { tm, tlm: tm });
            }
            EndText => {
                let o = self.text_object()?;
                reserve(
                    &mut self.result.text_objects,
                    1,
                    self.limits.max_text_objects,
                    &mut self.allocation,
                    self.limits.max_allocation_bytes,
                )?;
                self.event(TextEvent::EndTextObject(self.result.text_objects.len()))?;
                self.result.text_objects.push(TextObjectEnd {
                    text_matrix: o.tm,
                    line_matrix: o.tlm,
                    operation_index: self.source_index,
                    operation_span: self.operation_span.clone(),
                });
                self.object = None;
            }
            CharacterSpacing | WordSpacing | HorizontalScale | Leading | TextRise => {
                self.operand_index = Some(0);
                let n = self.text_value(number(&v[0])?)?;
                match operation.kind() {
                    CharacterSpacing => self.state.tc = n,
                    WordSpacing => self.state.tw = n,
                    HorizontalScale => self.state.h = self.text_value(n / 100.)?,
                    Leading => self.state.leading = n,
                    TextRise => self.state.rise = n,
                    _ => unreachable!(),
                }
            }
            Font => {
                self.operand_index = Some(1);
                let size = self.text_value(number(&v[1])?)?;
                self.operand_index = Some(0);
                let Value::Name(name) = &v[0] else {
                    unreachable!()
                };
                self.select_font(name, size, resolved_font)?;
            }
            TextRenderingMode => {
                let Value::Integer(n) = v[0] else {
                    unreachable!()
                };
                if n != 0 && n != 3 {
                    return Err(Failure::Unsupported(
                        "text rendering mode other than 0 or 3",
                    ));
                }
                self.state.tr = n as u8;
            }
            TextMove | TextMoveLeading => {
                self.operand_index = Some(0);
                let x = number(&v[0])?;
                self.operand_index = Some(1);
                let y = number(&v[1])?;
                if operation.kind() == TextMoveLeading {
                    self.state.leading = self.text_value(-y)?;
                }
                self.move_line(x, y)?;
            }
            TextNextLine => {
                self.move_line(0., -self.state.leading)?;
            }
            ShowText | NextLineShowText | SpacedNextLineShowText => {
                self.text_object()?;
                if self.state.font.is_none() {
                    return Err(Failure::State("no selected font"));
                }
                let operand = if operation.kind() == SpacedNextLineShowText {
                    self.operand_index = Some(0);
                    self.state.tw = self.text_value(number(&v[0])?)?;
                    self.operand_index = Some(1);
                    self.state.tc = self.text_value(number(&v[1])?)?;
                    2
                } else {
                    0
                };
                if operation.kind() != ShowText {
                    self.move_line(0., -self.state.leading)?;
                }
                self.operand_index = Some(operand);
                let Value::String(bytes) = &v[operand] else {
                    unreachable!()
                };
                self.show(bytes)?;
            }
            ShowTextArray => {
                self.text_object()?;
                if self.state.font.is_none() {
                    return Err(Failure::State("no selected font"));
                }
                self.operand_index = Some(0);
                let Value::Array(items) = &v[0] else {
                    unreachable!()
                };
                self.tj_elements = add_count(
                    self.tj_elements,
                    items.len(),
                    self.limits.max_tj_elements,
                    TextPositionLimitKind::TjElements,
                )?;
                // Preflight bounded element count and total payload before decoding any element.
                let mut bytes = 0;
                for (i, item) in items.iter().enumerate() {
                    self.array_index = Some(i);
                    match item {
                        Value::String(s) => {
                            bytes = add_count(
                                bytes,
                                s.len(),
                                self.limits.max_source_bytes - self.result.source_bytes.len(),
                                TextPositionLimitKind::SourceBytes,
                            )?;
                        }
                        _ => {
                            number(item)?;
                        }
                    }
                }
                self.shows = add_count(
                    self.shows,
                    1,
                    self.limits.max_show_operations,
                    TextPositionLimitKind::ShowOperations,
                )?;
                for (i, item) in items.iter().enumerate() {
                    self.array_index = Some(i);
                    match item {
                        Value::String(s) => self.show(s)?,
                        _ => self.adjust(number(item)?, i)?,
                    }
                }
            }
            _ => {
                return Err(Failure::Unsupported(
                    "operator outside standalone text subset",
                ));
            }
        }
        Ok(())
    }
    fn show(&mut self, bytes: &[u8]) -> ResultKind<()> {
        self.shows = add_count(
            self.shows,
            1,
            self.limits.max_show_operations,
            TextPositionLimitKind::ShowOperations,
        )?;
        add_count(
            self.result.source_bytes.len(),
            bytes.len(),
            self.limits.max_source_bytes,
            TextPositionLimitKind::SourceBytes,
        )?;
        add_count(
            self.result.codes.len(),
            bytes.len(),
            self.limits.max_codes,
            TextPositionLimitKind::Codes,
        )?;
        let id = self.state.font.ok_or(Failure::State("no selected font"))?;
        let font = self.fonts.by_id(id).ok_or(Failure::MissingFont)?;
        // Decode one byte at a time: temporary capacity is bounded independently
        // of entire string size, and repeated work is charged cumulatively. Each
        // Unicode sequence is at most the remaining caller allowance. D0 grows
        // geometrically; a factor four covers cumulative temporary requests.
        let source_start = self.result.source_bytes.len();
        reserve(
            &mut self.result.source_bytes,
            bytes.len(),
            self.limits.max_source_bytes,
            &mut self.allocation,
            self.limits.max_allocation_bytes,
        )?;
        self.result.source_bytes.extend_from_slice(bytes);
        let code_width = font.code_bytes();
        if bytes.len() % code_width != 0 {
            self.string_offset = Some(bytes.len() - bytes.len() % code_width);
            return Err(Failure::State("string ends inside a multi-byte code"));
        }
        for offset in (0..bytes.len()).step_by(code_width) {
            let code = bytes[offset..offset + code_width]
                .iter()
                .fold(0u32, |code, b| code << 8 | u32::from(*b));
            self.string_offset = Some(offset);
            let remaining_alloc = self.limits.max_allocation_bytes - self.allocation;
            let temp_code = size_of::<DecodedCode>()
                .checked_mul(4)
                .ok_or_else(|| limit(TextPositionLimitKind::AllocationBytes))?;
            if remaining_alloc < temp_code {
                return Err(limit(TextPositionLimitKind::AllocationBytes));
            }
            let remaining_utf8 = self.limits.max_utf8_bytes - self.result.text.len();
            let utf8_cap = remaining_utf8.min((remaining_alloc - temp_code) / 4);
            let decoded = font
                .decode(
                    &bytes[offset..offset + code_width],
                    TextDecodeLimits {
                        max_input_bytes: code_width,
                        max_codes: 1,
                        max_scalars: self.limits.max_scalars - self.scalars,
                        max_utf8_bytes: utf8_cap,
                    },
                )
                .map_err(|mut e| {
                    if utf8_cap < remaining_utf8
                        && matches!(
                            e.kind,
                            TextDecodeErrorKind::LimitExceeded(TextDecodeLimitKind::Utf8Bytes)
                        )
                    {
                        return limit(TextPositionLimitKind::AllocationBytes);
                    }
                    e.offset += offset;
                    Failure::Decode(e)
                })?;
            let temp_bytes = decoded
                .text()
                .len()
                .checked_mul(4)
                .and_then(|n| n.checked_add(temp_code))
                .ok_or_else(|| limit(TextPositionLimitKind::AllocationBytes))?;
            self.allocation = add_count(
                self.allocation,
                temp_bytes,
                self.limits.max_allocation_bytes,
                TextPositionLimitKind::AllocationBytes,
            )?;
            self.scalars = add_count(
                self.scalars,
                decoded.scalar_count(),
                self.limits.max_scalars,
                TextPositionLimitKind::Scalars,
            )?;
            let (advance, metric_origin) = font.advance(code).map_err(Failure::Font)?;
            let width = self.text_value(advance.value())?;
            let before = self.text_object()?.tm;
            let g = self.position_value(finite(width / 1000. * self.state.size)?)?;
            let g = self.position_value(finite(g + self.state.tc)?)?;
            let glyph_displacement = self.position_value(finite(g * self.state.h)?)?;
            // Tw applies only to the single-byte code 32 (ISO 32000 9.3.3).
            let word_spacing = if code_width == 1 && code == 0x20 {
                self.state.tw
            } else {
                0.
            };
            let word_displacement = self.position_value(finite(word_spacing * self.state.h)?)?;
            let mid = self.translate(before, glyph_displacement, 0.)?;
            let after = self.translate(mid, word_displacement, 0.)?;
            let sx = finite(self.state.size * self.state.h)?;
            let scale = matrix(
                Affine::<EmSpace, TextSpace>::from_pdf([
                    sx,
                    0.,
                    0.,
                    self.state.size,
                    0.,
                    self.state.rise,
                ])
                .map_err(|_| Failure::Nonfinite)?,
                self.limits,
            )?;
            let transform = compose(
                compose(scale, before, self.limits)?,
                self.state.ctm,
                self.limits,
            )?;
            let [_, _, _, _, x, y] = transform.coefficients();
            let origin = Point::new(x, y).map_err(|_| Failure::Nonfinite)?;
            let utf8_start = self.result.text.len();
            reserve_string(
                &mut self.result.text,
                decoded.text().len(),
                self.limits.max_utf8_bytes,
                &mut self.allocation,
                self.limits.max_allocation_bytes,
            )?;
            reserve(
                &mut self.result.codes,
                1,
                self.limits.max_codes,
                &mut self.allocation,
                self.limits.max_allocation_bytes,
            )?;
            self.event(TextEvent::Code(self.result.codes.len()))?;
            self.result.text.push_str(decoded.text());
            self.result.codes.push(CodePlacement {
                code,
                font_id: id,
                text_object: self.objects - 1,
                operation_index: self.source_index,
                operation_span: self.operation_span.clone(),
                operand_index: self.operand_index.unwrap_or(0),
                array_index: self.array_index,
                string_span: offset..offset + code_width,
                source_span: source_start + offset..source_start + offset + code_width,
                utf8_span: utf8_start..self.result.text.len(),
                width,
                metric_origin,
                character_spacing: self.state.tc,
                word_spacing,
                font_size: self.state.size,
                horizontal_scale: self.state.h,
                rise: self.state.rise,
                rendering_mode: self.state.tr,
                before,
                after,
                transform,
                origin,
                glyph_displacement,
                word_displacement,
            });
            self.object.as_mut().expect("checked text object").tm = after;
        }
        self.string_offset = None;
        Ok(())
    }
    fn adjust(&mut self, n: f64, index: usize) -> ResultKind<()> {
        self.text_value(n)?;
        let before = self.text_object()?.tm;
        let product = self.position_value(finite(-n / 1000. * self.state.size)?)?;
        let displacement = self.position_value(finite(product * self.state.h)?)?;
        let after = self.translate(before, displacement, 0.)?;
        reserve(
            &mut self.result.adjustments,
            1,
            self.limits.max_tj_elements,
            &mut self.allocation,
            self.limits.max_allocation_bytes,
        )?;
        self.event(TextEvent::Adjustment(self.result.adjustments.len()))?;
        self.result.adjustments.push(TextAdjustment {
            number: n,
            displacement,
            before,
            after,
            operation_index: self.source_index,
            operation_span: self.operation_span.clone(),
            array_index: index,
            text_object: self.objects - 1,
        });
        self.object.as_mut().expect("checked text object").tm = after;
        Ok(())
    }
}
/// Parse, validate and execute atomically. Unsupported mixed page operators fail.
pub fn position_text_content(
    input: &[u8],
    fonts: &TextFonts<'_>,
    initial: TextPositionInitialState,
    content_limits: ContentLimits,
    limits: TextPositionLimits,
) -> Result<PositionedText, TextPositionError> {
    let mut executor = TextPositioner::new(fonts, initial, limits)?;
    let mut parser = ContentParser::new(input, content_limits);
    loop {
        let operation = parser.next_operation().map_err(|e| {
            let mut result = executor.error(Failure::Content(Box::new(e)));
            result.operation_index = executor.operations;
            result
        })?;
        let Some(operation) = operation else {
            break;
        };
        // Bound TJ traversal before the general validator visits each element.
        if operation.operator == b"TJ" {
            if let Some(Value::Array(items)) = operation.operands.first() {
                if items.len() > limits.max_tj_elements.saturating_sub(executor.tj_elements) {
                    executor.operation_span = operation.span.clone();
                    let mut error = executor.error(limit(TextPositionLimitKind::TjElements));
                    error.operation_index = executor.operations;
                    return Err(error);
                }
            }
        }
        let validated = validate_operation(&operation).map_err(|e| TextPositionError {
            operation_index: executor.operations,
            operation_span: operation.span.clone(),
            operand_index: None,
            array_index: None,
            string_offset: None,
            kind: Failure::Operator(e),
        })?;
        executor.push(&validated)?;
    }
    executor.finish()
}
