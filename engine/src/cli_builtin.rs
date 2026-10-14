// SPDX-License-Identifier: AGPL-3.0-or-later
use super::append_inspection_output as append;
use tandem_pdf_engine::builtin_metrics::{BuiltinFace, BuiltinMetricLookup, BuiltinMetrics};

pub(super) fn inspect(bytes: &[u8], face: &str) -> Result<Vec<u8>, String> {
    let face = BuiltinFace::from_pdf_name(face.as_bytes()).ok_or("unsupported builtin face")?;
    let (_, encoding) = super::cli_encoding::load(bytes)?;
    let metrics = BuiltinMetrics::from_encoding(face, encoding);
    let mut output = Vec::new();
    append(
        &mut output,
        format!(
            "{{\"face\":\"{}\",\"glyphs\":[",
            std::str::from_utf8(face.pdf_name()).map_err(|_| "non-ASCII face constant")?
        )
        .as_bytes(),
    )?;
    for (index, (name, advance)) in face.glyphs().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        append(
            &mut output,
            format!(
                "{{\"name_hex\":\"{}\",\"advance\":{}}}",
                super::hex(name),
                advance.value()
            )
            .as_bytes(),
        )?;
    }
    append(&mut output, b"],\"slots\":")?;
    write_slots(&mut output, &metrics)?;
    append(&mut output, b"}\n")?;
    Ok(output)
}

pub(super) fn write_slots(output: &mut Vec<u8>, metrics: &BuiltinMetrics) -> Result<(), String> {
    append(output, b"[")?;
    for code in 0..=u8::MAX {
        if code > 0 {
            append(output, b",")?;
        }
        let glyph = metrics
            .glyph_name(code)
            .map(|name| format!("\"{}\"", super::hex(name)))
            .unwrap_or_else(|| "null".into());
        let (kind, advance) = match metrics.lookup(code) {
            BuiltinMetricLookup::UndefinedGlyph => ("undefined", "null".into()),
            BuiltinMetricLookup::MissingBuiltinMetric(_) => ("missing", "null".into()),
            BuiltinMetricLookup::Advance(value) => ("advance", value.value().to_string()),
        };
        append(output, format!("{{\"code\":{code},\"glyph_name_hex\":{glyph},\"kind\":\"{kind}\",\"advance\":{advance}}}").as_bytes())?;
    }
    append(output, b"]")?;
    Ok(())
}
