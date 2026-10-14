// SPDX-License-Identifier: AGPL-3.0-or-later
//! Typed operator classification and operand-shape validation, without graphics
//! execution. Resource-dependent color arity, font lookup, graphics/text state,
//! compatibility sections and resource expansion belong to a later interpreter.
use crate::content::{ContentOperand as Value, Operation};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatorKind {
    Save,
    Restore,
    Concat,
    LineWidth,
    LineCap,
    LineJoin,
    MiterLimit,
    Dash,
    RenderingIntent,
    Flatness,
    ExtGState,
    MoveTo,
    LineTo,
    CurveTo,
    CurveToV,
    CurveToY,
    ClosePath,
    Rectangle,
    Stroke,
    CloseStroke,
    Fill,
    EvenOddFill,
    FillStroke,
    EvenOddFillStroke,
    CloseFillStroke,
    CloseEvenOddFillStroke,
    EndPath,
    Clip,
    EvenOddClip,
    BeginText,
    EndText,
    CharacterSpacing,
    WordSpacing,
    HorizontalScale,
    Leading,
    Font,
    TextRenderingMode,
    TextRise,
    TextMove,
    TextMoveLeading,
    TextMatrix,
    TextNextLine,
    ShowText,
    ShowTextArray,
    NextLineShowText,
    SpacedNextLineShowText,
    Type3Width,
    Type3WidthBounds,
    StrokeColorSpace,
    FillColorSpace,
    StrokeColor,
    FillColor,
    StrokeColorPattern,
    FillColorPattern,
    StrokeGray,
    FillGray,
    StrokeRgb,
    FillRgb,
    StrokeCmyk,
    FillCmyk,
    Shading,
    XObject,
    MarkedPoint,
    MarkedPointProperties,
    BeginMarkedContent,
    BeginMarkedContentProperties,
    EndMarkedContent,
    BeginCompatibility,
    EndCompatibility,
    /// `BI ... ID ... EI`, parsed as a dictionary and a data string.
    InlineImage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperatorErrorKind {
    UnknownOperator,
    InlineImageUnsupported,
    Arity,
    OperandType,
    NonfiniteNumber,
    Range,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorError {
    pub offset: usize,
    pub kind: OperatorErrorKind,
}
impl fmt::Display for OperatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PDF content operator error at byte {}: {:?}",
            self.offset, self.kind
        )
    }
}
impl std::error::Error for OperatorError {}

/// Only constructible through validation. This borrows operands, including
/// strings and text arrays, without duplicating payload or allocating a list.
#[derive(Debug)]
pub struct ValidatedOperation<'a> {
    kind: OperatorKind,
    operation: &'a Operation,
}
impl<'a> ValidatedOperation<'a> {
    pub fn kind(&self) -> OperatorKind {
        self.kind
    }
    pub fn operation(&self) -> &'a Operation {
        self.operation
    }
    pub fn operands(&self) -> &'a [Value] {
        &self.operation.operands
    }
}

fn number(value: &Value) -> Result<f64, OperatorErrorKind> {
    let n = match value {
        Value::Integer(n) => *n as f64,
        Value::Real(n) => *n,
        _ => return Err(OperatorErrorKind::OperandType),
    };
    if n.is_finite() {
        Ok(n)
    } else {
        Err(OperatorErrorKind::NonfiniteNumber)
    }
}
fn arity(values: &[Value], count: usize) -> Result<(), OperatorErrorKind> {
    if values.len() == count {
        Ok(())
    } else {
        Err(OperatorErrorKind::Arity)
    }
}
fn numbers(values: &[Value], count: usize) -> Result<(), OperatorErrorKind> {
    arity(values, count)?;
    for value in values {
        number(value)?;
    }
    Ok(())
}
fn name(value: &Value) -> Result<(), OperatorErrorKind> {
    if matches!(value, Value::Name(_)) {
        Ok(())
    } else {
        Err(OperatorErrorKind::OperandType)
    }
}
fn string(value: &Value) -> Result<(), OperatorErrorKind> {
    if matches!(value, Value::String(_)) {
        Ok(())
    } else {
        Err(OperatorErrorKind::OperandType)
    }
}
fn integer_range(value: &Value, max: i64) -> Result<(), OperatorErrorKind> {
    match value {
        Value::Integer(n) if (0..=max).contains(n) => Ok(()),
        Value::Integer(_) => Err(OperatorErrorKind::Range),
        _ => Err(OperatorErrorKind::OperandType),
    }
}
fn variable_color(values: &[Value], pattern: bool) -> Result<(), OperatorErrorKind> {
    if values.is_empty() {
        return Err(OperatorErrorKind::Arity);
    }
    let numeric = if pattern && matches!(values.last(), Some(Value::Name(_))) {
        &values[..values.len() - 1]
    } else {
        values
    };
    if numeric.len() > 32 {
        return Err(OperatorErrorKind::Range);
    }
    for value in numeric {
        number(value)?;
    }
    Ok(())
}
fn classify(op: &Operation) -> Result<OperatorKind, OperatorErrorKind> {
    use OperatorKind::*;
    let values = op.operands.as_slice();
    let (kind, count) = match op.operator.as_slice() {
        b"q" => (Save, 0),
        b"Q" => (Restore, 0),
        b"cm" => (Concat, 6),
        b"w" => (LineWidth, 1),
        b"J" => (LineCap, 1),
        b"j" => (LineJoin, 1),
        b"M" => (MiterLimit, 1),
        b"d" => (Dash, 2),
        b"ri" => (RenderingIntent, 1),
        b"i" => (Flatness, 1),
        b"gs" => (ExtGState, 1),
        b"m" => (MoveTo, 2),
        b"l" => (LineTo, 2),
        b"c" => (CurveTo, 6),
        b"v" => (CurveToV, 4),
        b"y" => (CurveToY, 4),
        b"h" => (ClosePath, 0),
        b"re" => (Rectangle, 4),
        b"S" => (Stroke, 0),
        b"s" => (CloseStroke, 0),
        b"f" | b"F" => (Fill, 0),
        b"f*" => (EvenOddFill, 0),
        b"B" => (FillStroke, 0),
        b"B*" => (EvenOddFillStroke, 0),
        b"b" => (CloseFillStroke, 0),
        b"b*" => (CloseEvenOddFillStroke, 0),
        b"n" => (EndPath, 0),
        b"W" => (Clip, 0),
        b"W*" => (EvenOddClip, 0),
        b"BT" => (BeginText, 0),
        b"ET" => (EndText, 0),
        b"Tc" => (CharacterSpacing, 1),
        b"Tw" => (WordSpacing, 1),
        b"Tz" => (HorizontalScale, 1),
        b"TL" => (Leading, 1),
        b"Tf" => (Font, 2),
        b"Tr" => (TextRenderingMode, 1),
        b"Ts" => (TextRise, 1),
        b"Td" => (TextMove, 2),
        b"TD" => (TextMoveLeading, 2),
        b"Tm" => (TextMatrix, 6),
        b"T*" => (TextNextLine, 0),
        b"Tj" => (ShowText, 1),
        b"TJ" => (ShowTextArray, 1),
        b"'" => (NextLineShowText, 1),
        b"\"" => (SpacedNextLineShowText, 3),
        b"d0" => (Type3Width, 2),
        b"d1" => (Type3WidthBounds, 6),
        b"CS" => (StrokeColorSpace, 1),
        b"cs" => (FillColorSpace, 1),
        b"SC" => {
            variable_color(values, false)?;
            return Ok(StrokeColor);
        }
        b"sc" => {
            variable_color(values, false)?;
            return Ok(FillColor);
        }
        b"SCN" => {
            variable_color(values, true)?;
            return Ok(StrokeColorPattern);
        }
        b"scn" => {
            variable_color(values, true)?;
            return Ok(FillColorPattern);
        }
        b"G" => (StrokeGray, 1),
        b"g" => (FillGray, 1),
        b"RG" => (StrokeRgb, 3),
        b"rg" => (FillRgb, 3),
        b"K" => (StrokeCmyk, 4),
        b"k" => (FillCmyk, 4),
        b"sh" => (Shading, 1),
        b"Do" => (XObject, 1),
        b"MP" => (MarkedPoint, 1),
        b"DP" => (MarkedPointProperties, 2),
        b"BMC" => (BeginMarkedContent, 1),
        b"BDC" => (BeginMarkedContentProperties, 2),
        b"EMC" => (EndMarkedContent, 0),
        b"BX" => (BeginCompatibility, 0),
        b"EX" => (EndCompatibility, 0),
        b"BI" => {
            arity(values, 2)?;
            if !matches!(values, [Value::Dictionary(_), Value::String(_)]) {
                return Err(OperatorErrorKind::OperandType);
            }
            return Ok(InlineImage);
        }
        b"ID" | b"EI" => return Err(OperatorErrorKind::InlineImageUnsupported),
        _ => return Err(OperatorErrorKind::UnknownOperator),
    };
    arity(values, count)?;
    match kind {
        RenderingIntent | ExtGState | StrokeColorSpace | FillColorSpace | Shading | XObject
        | MarkedPoint | BeginMarkedContent => name(&values[0])?,
        Font => {
            name(&values[0])?;
            number(&values[1])?;
        }
        MarkedPointProperties | BeginMarkedContentProperties => {
            name(&values[0])?;
            if !matches!(values[1], Value::Name(_) | Value::Dictionary(_)) {
                return Err(OperatorErrorKind::OperandType);
            }
        }
        ShowText | NextLineShowText => string(&values[0])?,
        SpacedNextLineShowText => {
            number(&values[0])?;
            number(&values[1])?;
            string(&values[2])?;
        }
        ShowTextArray => {
            let Value::Array(items) = &values[0] else {
                return Err(OperatorErrorKind::OperandType);
            };
            for item in items {
                if !matches!(item, Value::String(_)) {
                    number(item)?;
                }
            }
        }
        LineCap | LineJoin => integer_range(&values[0], 2)?,
        TextRenderingMode => integer_range(&values[0], 7)?,
        LineWidth | MiterLimit | Flatness => {
            let n = number(&values[0])?;
            if (kind == LineWidth && n < 0.0)
                || (kind == MiterLimit && n < 1.0)
                || (kind == Flatness && !(0.0..=100.0).contains(&n))
            {
                return Err(OperatorErrorKind::Range);
            }
        }
        Dash => {
            let Value::Array(items) = &values[0] else {
                return Err(OperatorErrorKind::OperandType);
            };
            let mut positive = false;
            for item in items {
                let n = number(item)?;
                if n < 0.0 {
                    return Err(OperatorErrorKind::Range);
                }
                positive |= n > 0.0;
            }
            if (!items.is_empty() && !positive) || number(&values[1])? < 0.0 {
                return Err(OperatorErrorKind::Range);
            }
        }
        _ => numbers(values, count)?,
    }
    Ok(kind)
}

/// Validate local arity/types/ranges only. No allocation or payload cloning.
/// This does not authorize execution of unknown operators inside BX/EX, enforce
/// text/graphics state balance, or resolve resource-dependent color dimensions.
pub fn validate_operation(operation: &Operation) -> Result<ValidatedOperation<'_>, OperatorError> {
    classify(operation)
        .map(|kind| ValidatedOperation { kind, operation })
        .map_err(|kind| OperatorError {
            offset: operation.span.start,
            kind,
        })
}
