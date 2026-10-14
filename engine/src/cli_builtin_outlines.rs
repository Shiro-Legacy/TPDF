// SPDX-License-Identifier: AGPL-3.0-or-later
use super::{append_inspection_output as append, cli_page_program::string};
use tandem_pdf_engine::{
    builtin_metrics::BuiltinFace,
    builtin_outlines::{BuiltinOutline, BuiltinOutlineCommand},
};
pub(super) fn inspect(selector: &str) -> Result<Vec<u8>, String> {
    let face =
        BuiltinFace::from_pdf_name(selector.as_bytes()).ok_or("unsupported pinned outline face")?;
    let mut out = Vec::new();
    append(&mut out, b"{\"face\":")?;
    string(&mut out, selector)?;
    append(
        &mut out,
        format!(
            ",\"static_bytes\":{},\"glyphs\":[",
            BuiltinOutline::static_bytes()
        )
        .as_bytes(),
    )?;
    for (i, glyph) in BuiltinOutline::glyphs(face).enumerate() {
        if i != 0 {
            append(&mut out, b",")?;
        }
        append(&mut out, b"{\"name\":")?;
        string(
            &mut out,
            std::str::from_utf8(glyph.name()).map_err(|_| "non-UTF8 pinned glyph name")?,
        )?;
        let range = glyph.command_range();
        let advance = face
            .lookup_glyph(glyph.name())
            .ok_or("outline has no pinned metric")?
            .value();
        append(&mut out,format!(",\"native_gid\":{},\"units_per_em\":{},\"advance\":{},\"asset_sha256\":\"{}\",\"command_range\":[{},{}],\"commands\":[",glyph.native_gid(),glyph.units_per_em(),advance,glyph.asset_sha256(),range.start,range.end).as_bytes())?;
        for (j, command) in glyph.commands().iter().enumerate() {
            if j != 0 {
                append(&mut out, b",")?;
            }
            let value = match command {
                BuiltinOutlineCommand::MoveTo { x, y } => format!("[\"m\",{x},{y}]"),
                BuiltinOutlineCommand::LineTo { x, y } => format!("[\"l\",{x},{y}]"),
                BuiltinOutlineCommand::CubicTo {
                    x1,
                    y1,
                    x2,
                    y2,
                    x,
                    y,
                } => format!("[\"c\",{x1},{y1},{x2},{y2},{x},{y}]"),
                BuiltinOutlineCommand::Close => "[\"h\"]".into(),
            };
            append(&mut out, value.as_bytes())?;
        }
        append(&mut out, b"]}")?;
    }
    append(&mut out, b"]}\n")?;
    Ok(out)
}
