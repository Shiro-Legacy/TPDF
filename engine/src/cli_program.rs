// SPDX-License-Identifier: AGPL-3.0-or-later
//! Inspect raw child arenas and the exact interleaved coordinator event order.
use super::{append_inspection_output as append, cli_path};
use tandem_pdf_engine::content::ContentLimits;
use tandem_pdf_engine::path::{
    InitialClip, LineCap, LineJoin, PathEvent, PathProgram, StrokeStyle,
};
use tandem_pdf_engine::program::{
    Program, ProgramEventKind, ProgramInitialState, ProgramLimits, interpret_program_content,
};

fn index(value: Option<usize>) -> String {
    value
        .map(|n| n.to_string())
        .unwrap_or_else(|| "null".into())
}
fn span(value: std::ops::Range<usize>) -> String {
    format!("[{},{}]", value.start, value.end)
}
fn style(output: &mut Vec<u8>, value: StrokeStyle) -> Result<(), String> {
    let cap = match value.cap() {
        LineCap::Butt => 0,
        LineCap::Round => 1,
        LineCap::Square => 2,
    };
    let join = match value.join() {
        LineJoin::Miter => 0,
        LineJoin::Round => 1,
        LineJoin::Bevel => 2,
    };
    append(output, format!(",\"style\":{{\"width\":{},\"cap\":{cap},\"join\":{join},\"miter\":{},\"dash\":{},\"phase\":{}}}",value.width(),value.miter_limit(),index(value.dash().map(|v|v.index())),value.dash_phase()).as_bytes())
}
pub(super) fn paths_json(output: &mut Vec<u8>, program: &PathProgram) -> Result<(), String> {
    append(
        output,
        format!(
            "{{\"initial_ctm\":{:?},\"initial_clip\":",
            program.initial_ctm().coefficients()
        )
        .as_bytes(),
    )?;
    match program.initial_clip() {
        InitialClip::Unbounded => append(output, b"{\"kind\":\"unbounded\"}")?,
        InitialClip::Empty => append(output, b"{\"kind\":\"empty\"}")?,
        InitialClip::Rectangle(rect) => append(
            output,
            format!("{{\"kind\":\"rectangle\",\"bounds\":{:?}}}", rect.bounds()).as_bytes(),
        )?,
    }
    append(output, b",\"paths\":[")?;
    for id in 0..program.paths().len() {
        if id > 0 {
            append(output, b",")?;
        }
        cli_path::path(output, program, id)?;
    }
    append(output, b"],\"clips\":[")?;
    for (id, clip) in program.clips().iter().enumerate() {
        if id > 0 {
            append(output, b",")?;
        }
        append(
            output,
            format!(
                "{{\"parent\":{},\"path\":{},\"rule\":\"{}\",\"matrix\":{:?}}}",
                index(clip.parent().map(|v| v.index())),
                clip.path().index(),
                cli_path::rule(clip.rule()),
                clip.ctm().coefficients()
            )
            .as_bytes(),
        )?;
    }
    append(output, b"],\"dashes\":[")?;
    for (id, dash) in program.dashes().iter().enumerate() {
        if id > 0 {
            append(output, b",")?;
        }
        cli_path::numbers(output, dash.values())?;
    }
    append(output, b"],\"events\":[")?;
    for (id, event) in program.events().iter().enumerate() {
        if id > 0 {
            append(output, b",")?;
        }
        match event {
            PathEvent::Fill { paint, .. } | PathEvent::Stroke { paint, .. } => {
                let kind = if matches!(event, PathEvent::Fill { .. }) {
                    "fill"
                } else {
                    "stroke"
                };
                append(
                    output,
                    format!(
                        "{{\"kind\":\"{kind}\",\"path\":{},\"matrix\":{:?},\"clip\":{},\"span\":{}",
                        paint.path().index(),
                        paint.ctm().coefficients(),
                        index(paint.clip().map(|v| v.index())),
                        span(paint.span())
                    )
                    .as_bytes(),
                )?;
                cli_path::color(output, paint.color())?;
                match event {
                    PathEvent::Fill { rule, .. } => append(
                        output,
                        format!(",\"rule\":\"{}\"", cli_path::rule(*rule)).as_bytes(),
                    )?,
                    PathEvent::Stroke { style: value, .. } => style(output, *value)?,
                    _ => unreachable!(),
                }
                append(output, b"}")?;
            }
            PathEvent::ClipPush { id, span: source } => append(
                output,
                format!(
                    "{{\"kind\":\"clip\",\"clip\":{},\"span\":{}}}",
                    id.index(),
                    span(source.clone())
                )
                .as_bytes(),
            )?,
            PathEvent::ClipRestore { clip, span: source } => append(
                output,
                format!(
                    "{{\"kind\":\"restore\",\"clip\":{},\"span\":{}}}",
                    index(clip.map(|v| v.index())),
                    span(source.clone())
                )
                .as_bytes(),
            )?,
        }
    }
    append(output, b"]}")
}
pub(super) fn trace_json(program: &Program) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    append(&mut output, b"{\"paths\":")?;
    paths_json(&mut output, program.paths())?;
    append(&mut output, b",\"text\":")?;
    let text = super::cli_text::trace_json(program.text())?;
    append(&mut output, &text)?;
    append(&mut output, b",\"text_contexts\":[")?;
    for (id, context) in program.text_contexts().iter().enumerate() {
        if id > 0 {
            append(&mut output, b",")?;
        }
        append(
            &mut output,
            format!(
                "{{\"matrix\":{:?},\"clip\":{}",
                context.ctm().coefficients(),
                index(context.clip().map(|v| v.index()))
            )
            .as_bytes(),
        )?;
        cli_path::color(&mut output, context.fill_color())?;
        append(&mut output, b"}")?;
    }
    append(&mut output, b"],\"events\":[")?;
    for (id, event) in program.events().iter().enumerate() {
        if id > 0 {
            append(&mut output, b",")?;
        }
        let (kind, child, context) = match event.kind() {
            ProgramEventKind::Image(index) => ("image", index, None),
            ProgramEventKind::Path(index) => ("path", index, None),
            ProgramEventKind::Text { event, context } => ("text", event, Some(context)),
        };
        append(&mut output,format!("{{\"operation_index\":{},\"span\":{},\"kind\":\"{kind}\",\"event\":{child},\"context\":{}}}",event.operation_index(),span(event.span()),index(context)).as_bytes())?;
    }
    let usage = program.usage();
    append(&mut output,format!("],\"usage\":{{\"operations\":{},\"child_operations\":{},\"work_units\":{},\"allocated_bytes\":{}}}}}\n",usage.operations,usage.child_operations,usage.work_units,usage.allocated_bytes).as_bytes())?;
    Ok(output)
}
pub(super) fn inspect(input: &[u8], font_path: &str) -> Result<Vec<u8>, String> {
    super::cli_text::with_fonts(font_path, |fonts| {
        let program = interpret_program_content(
            input,
            fonts,
            ProgramInitialState::default(),
            ContentLimits::default(),
            ProgramLimits::default(),
        )
        .map_err(|e| e.to_string())?;
        trace_json(&program)
    })
}
