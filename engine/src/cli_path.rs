// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded native-compatible trace of a complete standalone path program.
use super::append_inspection_output as append;
use tandem_pdf_engine::content::ContentLimits;
use tandem_pdf_engine::path::{
    ClipId, DeviceColor, FillRule, LineCap, LineJoin, PathCommand, PathEvent, PathInitialState,
    PathLimits, PathProgram, interpret_path_content,
};
pub(super) fn numbers(output: &mut Vec<u8>, values: &[f64]) -> Result<(), String> {
    append(output, b"[")?;
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            append(output, b",")?;
        }
        append(output, v.to_string().as_bytes())?;
    }
    append(output, b"]")
}
pub(super) fn path(output: &mut Vec<u8>, program: &PathProgram, id: usize) -> Result<(), String> {
    append(output, b"[")?;
    for (i, command) in program
        .paths()
        .get(id)
        .ok_or("path ID out of bounds")?
        .commands()
        .iter()
        .enumerate()
    {
        if i > 0 {
            append(output, b",")?;
        }
        match command {
            PathCommand::MoveTo(p) => {
                append(output, format!("[\"m\",{},{}]", p.x(), p.y()).as_bytes())?
            }
            PathCommand::LineTo(p) => {
                append(output, format!("[\"l\",{},{}]", p.x(), p.y()).as_bytes())?
            }
            PathCommand::CubicTo {
                control1: a,
                control2: b,
                end: c,
            } => append(
                output,
                format!(
                    "[\"c\",{},{},{},{},{},{}]",
                    a.x(),
                    a.y(),
                    b.x(),
                    b.y(),
                    c.x(),
                    c.y()
                )
                .as_bytes(),
            )?,
            PathCommand::ClosePath => append(output, b"[\"h\"]")?,
        }
    }
    append(output, b"]")
}
pub(super) fn color(output: &mut Vec<u8>, value: DeviceColor) -> Result<(), String> {
    let (space, values) = match value {
        DeviceColor::Gray(v) => ("gray", vec![v]),
        DeviceColor::Rgb(v) => ("rgb", v.to_vec()),
        DeviceColor::Cmyk(v) => ("cmyk", v.to_vec()),
    };
    append(
        output,
        format!(",\"color\":{{\"space\":\"{space}\",\"values\":").as_bytes(),
    )?;
    numbers(output, &values)?;
    append(output, b"}")
}
pub(super) fn rule(value: FillRule) -> &'static str {
    match value {
        FillRule::NonZero => "nonzero",
        FillRule::EvenOdd => "evenodd",
    }
}
fn start(output: &mut Vec<u8>, count: &mut usize, kind: &str, depth: usize) -> Result<(), String> {
    if *count > 0 {
        append(output, b",")?;
    }
    *count += 1;
    append(
        output,
        format!("{{\"kind\":\"{kind}\",\"clip_depth\":{depth}").as_bytes(),
    )
}
pub(super) fn inspect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let program = interpret_path_content(
        bytes,
        PathInitialState::default(),
        ContentLimits::default(),
        PathLimits::default(),
    )
    .map_err(|e| e.to_string())?;
    let mut output = Vec::new();
    let mut current: Option<ClipId> = None;
    let mut depth = 0usize;
    let mut count = 0usize;
    append(&mut output, b"{\"events\":[")?;
    for event in program.events() {
        match event {
            PathEvent::Fill { paint, .. } | PathEvent::Stroke { paint, .. } => {
                if paint.clip() != current {
                    return Err("paint clip event invariant failed".into());
                }
                let kind = if matches!(event, PathEvent::Fill { .. }) {
                    "fill"
                } else {
                    "stroke"
                };
                start(&mut output, &mut count, kind, depth)?;
                append(&mut output, b",\"path\":")?;
                path(&mut output, &program, paint.path().index())?;
                append(&mut output, b",\"matrix\":")?;
                numbers(&mut output, &paint.ctm().coefficients())?;
                color(&mut output, paint.color())?;
                match event {
                    PathEvent::Fill {
                        rule: fill_rule, ..
                    } => append(
                        &mut output,
                        format!(",\"rule\":\"{}\"", rule(*fill_rule)).as_bytes(),
                    )?,
                    PathEvent::Stroke { style, .. } => {
                        let cap = match style.cap() {
                            LineCap::Butt => 0,
                            LineCap::Round => 1,
                            LineCap::Square => 2,
                        };
                        let join = match style.join() {
                            LineJoin::Miter => 0,
                            LineJoin::Round => 1,
                            LineJoin::Bevel => 2,
                        };
                        append(&mut output,format!(",\"style\":{{\"width\":{},\"start_cap\":{cap},\"dash_cap\":{cap},\"end_cap\":{cap},\"join\":{join},\"miter\":{},\"phase\":{},\"dash\":",style.width(),style.miter_limit(),style.dash_phase()).as_bytes())?;
                        let dash = if let Some(id) = style.dash() {
                            program
                                .dashes()
                                .get(id.index())
                                .ok_or("dash ID out of bounds")?
                                .values()
                        } else {
                            &[]
                        };
                        numbers(&mut output, dash)?;
                        append(&mut output, b"}")?;
                    }
                    _ => unreachable!(),
                }
                append(&mut output, b"}")?;
            }
            PathEvent::ClipPush { id, .. } => {
                let node = program
                    .clips()
                    .get(id.index())
                    .ok_or("clip ID out of bounds")?;
                if node.parent() != current {
                    return Err("clip parent event invariant failed".into());
                }
                start(&mut output, &mut count, "clip", depth)?;
                append(&mut output, b",\"path\":")?;
                path(&mut output, &program, node.path().index())?;
                append(&mut output, b",\"matrix\":")?;
                numbers(&mut output, &node.ctm().coefficients())?;
                append(
                    &mut output,
                    format!(",\"rule\":\"{}\"}}", rule(node.rule())).as_bytes(),
                )?;
                current = Some(*id);
                depth += 1;
            }
            PathEvent::ClipRestore { clip, .. } => {
                while current != *clip {
                    let id = current.ok_or("restored clip is not an ancestor")?;
                    current = program
                        .clips()
                        .get(id.index())
                        .ok_or("clip ID out of bounds")?
                        .parent();
                    depth = depth.checked_sub(1).ok_or("clip depth underflow")?;
                    start(&mut output, &mut count, "pop", depth)?;
                    append(&mut output, b"}")?;
                }
            }
        }
    }
    append(&mut output, b"]}\n")?;
    Ok(output)
}
