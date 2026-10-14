// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exact unhinted TrueType geometry for explicit glyph IDs, not PDF glyph selection.
use super::append_inspection_output as append;
use std::{fs::File, io::Read};
use tandem_pdf_engine::truetype_outline::{
    FontLimits, GlyphBounds, GlyphOutline, OutlineCommand, OutlineKind, OutlineLimits, TrueTypeFont,
};

fn queries(path: &str, count: u16) -> Result<Vec<u32>, String> {
    if path == "all" {
        return Ok((0..u32::from(count)).collect());
    }
    const LIMIT: usize = 1024 * 1024;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| format!("open outline query file: {e}"))?
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > LIMIT {
        return Err("outline query file exceeds 1 MiB".into());
    }
    if !bytes.is_ascii() {
        return Err("outline queries must be ASCII decimal glyph IDs".into());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| "outline queries must be ASCII decimal glyph IDs")?;
    let mut result = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if result.len() >= 65536 {
            return Err("outline queries exceed 65536".into());
        }
        if !line.as_bytes().iter().all(u8::is_ascii_digit) {
            return Err("outline queries require one unsigned decimal glyph ID per line".into());
        }
        result.push(
            line.parse::<u32>()
                .map_err(|_| "outline glyph ID exceeds u32")?,
        );
    }
    Ok(result)
}
fn bounds_json(bounds: Option<GlyphBounds>) -> String {
    bounds.map_or_else(|| "null".into(), |b| format!("{:?}", b.coordinates()))
}
pub(super) fn outline_json(output: &mut Vec<u8>, glyph: &GlyphOutline) -> Result<(), String> {
    let kind = match glyph.kind() {
        OutlineKind::EmptyLocaRange => "empty_loca",
        OutlineKind::ZeroContours => "zero_contours",
        OutlineKind::Simple => "simple",
        OutlineKind::Composite => "composite",
    };
    let range = glyph.glyph_range();
    append(output,format!("{{\"gid\":{},\"kind\":\"{kind}\",\"range\":[{},{}],\"instruction_bytes\":{},\"overlap\":{},\"points\":[",glyph.id().get(),range.start,range.end,glyph.instruction_bytes(),glyph.overlap()).as_bytes())?;
    for (index, point) in glyph.points().iter().enumerate() {
        if index > 0 {
            append(output, b",")?;
        }
        append(
            output,
            format!("[{},{},{}]", point.x(), point.y(), point.on_curve()).as_bytes(),
        )?;
    }
    append(
        output,
        format!(
            "],\"contour_endpoints\":{:?},\"commands\":[",
            glyph.contour_endpoints()
        )
        .as_bytes(),
    )?;
    for (index, command) in glyph.commands().iter().enumerate() {
        if index > 0 {
            append(output, b",")?;
        }
        match command {
            OutlineCommand::MoveTo(point) => append(
                output,
                format!("[\"m\",{},{}]", point.x2(), point.y2()).as_bytes(),
            )?,
            OutlineCommand::LineTo(point) => append(
                output,
                format!("[\"l\",{},{}]", point.x2(), point.y2()).as_bytes(),
            )?,
            OutlineCommand::QuadTo { control, end } => append(
                output,
                format!(
                    "[\"q\",{},{},{},{}]",
                    control.x2(),
                    control.y2(),
                    end.x2(),
                    end.y2()
                )
                .as_bytes(),
            )?,
            OutlineCommand::Close => append(output, b"[\"h\"]")?,
        }
    }
    append(
        output,
        format!(
            "],\"header_bounds\":{},\"control_bounds\":{}}}",
            bounds_json(glyph.header_bounds()),
            bounds_json(glyph.control_bounds())
        )
        .as_bytes(),
    )
}
pub(super) fn inspect(bytes: &[u8], query_path: &str) -> Result<Vec<u8>, String> {
    let font = TrueTypeFont::open(bytes, FontLimits::default()).map_err(|e| e.to_string())?;
    let queries = queries(query_path, font.glyph_count())?;
    let mut session = font.session(OutlineLimits::default());
    let mut output = Vec::new();
    append(
        &mut output,
        format!(
            "{{\"units_per_em\":{},\"glyph_count\":{},\"glyphs\":[",
            font.units_per_em(),
            font.glyph_count()
        )
        .as_bytes(),
    )?;
    for (index, raw) in queries.into_iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        let id = font.glyph_id(raw).map_err(|e| e.to_string())?;
        let glyph = session.decode(id).map_err(|e| e.to_string())?;
        outline_json(&mut output, &glyph)?;
    }
    append(&mut output, b"],\"tables\":[")?;
    for (index, table) in font.table_records().iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        append(
            &mut output,
            format!(
                "{{\"tag_bytes\":{:?},\"offset\":{},\"length\":{},\"checksum\":{}}}",
                table.tag(),
                table.offset(),
                table.length(),
                table.checksum()
            )
            .as_bytes(),
        )?;
    }
    let usage = session.usage();
    let font_usage = font.usage();
    append(&mut output, format!("],\"font_usage\":{{\"index_allocation_bytes\":{},\"work\":{}}},\"outline_usage\":{{\"decode_attempts\":{},\"glyph_bytes\":{},\"contours\":{},\"points\":{},\"commands\":{},\"allocation_bytes\":{},\"work\":{}}}}}\n", font_usage.index_allocation_bytes, font_usage.work, usage.decode_attempts, usage.glyph_bytes, usage.contours, usage.points, usage.commands, usage.allocation_bytes, usage.work).as_bytes())?;
    Ok(output)
}
