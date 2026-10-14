// SPDX-License-Identifier: AGPL-3.0-or-later
//! Standalone explicit-font text trace. This does not load a PDF page's fonts.
use super::append_inspection_output as append;
use std::{fs::File, io::Read};
use tandem_pdf_engine::builtin_metrics::{BuiltinFace, BuiltinMetrics};
use tandem_pdf_engine::cmap::{CMapLimits, ToUnicodeMap, parse_to_unicode};
use tandem_pdf_engine::content::ContentLimits;
use tandem_pdf_engine::font_metrics::{AdvanceOrigin, FontAdvance, HorizontalMetrics};
use tandem_pdf_engine::object::Object;
use tandem_pdf_engine::parser::{ParseLimits, parse_complete};
use tandem_pdf_engine::simple_encoding::SimpleEncoding;
use tandem_pdf_engine::text_font::{
    TextFont, TextFontLimits, TextFonts, TextMetricKind, TextMetricOrigin, TextUnicodeKind,
    UnicodeSource,
};
use tandem_pdf_engine::text_position::{
    PositionedText, TextEvent, TextPositionInitialState, TextPositionLimits, position_text_content,
};

struct FontData {
    name: Vec<u8>,
    unicode: Option<ToUnicodeMap>,
    encoding: Option<SimpleEncoding>,
    declared: Option<HorizontalMetrics>,
    builtin: Option<BuiltinMetrics>,
}
fn number(value: &Object) -> Result<FontAdvance, String> {
    match value {
        Object::Integer(v) => FontAdvance::from_integer(*v),
        Object::Real(v) => FontAdvance::new(*v),
        _ => return Err("width must be a finite direct number".into()),
    }
    .map_err(|e| e.to_string())
}
fn byte(value: Option<&Object>) -> Result<u8, String> {
    match value {
        Some(Object::Integer(v)) => {
            u8::try_from(*v).map_err(|_| "font range endpoint must be in 0..255".into())
        }
        _ => Err("font range endpoint must be a direct integer".into()),
    }
}
fn load_fonts(path: &str) -> Result<Vec<FontData>, String> {
    const MAX_CONFIG: usize = 1024 * 1024;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| format!("open text font configuration: {e}"))?
        .take(MAX_CONFIG as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_CONFIG {
        return Err("text font configuration exceeds 1 MiB".into());
    }
    let limits = ParseLimits {
        max_decoded_bytes: MAX_CONFIG,
        max_depth: 8,
        ..ParseLimits::default()
    };
    let Object::Dictionary(fonts) = parse_complete(&bytes, limits).map_err(|e| e.to_string())?
    else {
        return Err("font configuration must be a dictionary".into());
    };
    if fonts.len() > 256 {
        return Err("font configuration exceeds 256 bindings".into());
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(fonts.len())
        .map_err(|_| "font configuration allocation failed")?;
    let mut total_name_bytes = 0usize;
    for (name, value) in fonts {
        if name.len() > 127 {
            return Err("font name exceeds 127 bytes".into());
        }
        total_name_bytes = total_name_bytes
            .checked_add(name.len())
            .filter(|n| *n <= 32 * 1024)
            .ok_or("font names exceed 32 KiB")?;
        let Object::Dictionary(mut spec) = value else {
            return Err("each configured font must be a dictionary".into());
        };
        if spec.keys().any(|key| {
            !matches!(
                key.as_slice(),
                b"Encoding"
                    | b"ToUnicode"
                    | b"FirstChar"
                    | b"LastChar"
                    | b"Widths"
                    | b"MissingWidth"
                    | b"Builtin"
            )
        }) {
            return Err("unknown font configuration key".into());
        }
        let unicode = match spec.remove(b"ToUnicode".as_slice()) {
            None => None,
            Some(Object::String(source)) => Some(
                parse_to_unicode(
                    &source,
                    CMapLimits {
                        max_input_bytes: 64 * 1024,
                        max_mappings: 256,
                        max_owned_bytes: 64 * 1024,
                        max_records: 512,
                        ..CMapLimits::default()
                    },
                )
                .map_err(|e| e.to_string())?,
            ),
            _ => return Err("ToUnicode must be a direct byte string".into()),
        };
        let encoding = spec
            .remove(b"Encoding".as_slice())
            .map(|v| super::cli_encoding::load_object(&v).map(|(_, encoding)| encoding))
            .transpose()?;
        let mut data = FontData {
            name,
            unicode,
            encoding,
            declared: None,
            builtin: None,
        };
        if let Some(face) = spec.remove(b"Builtin".as_slice()) {
            if !spec.is_empty() {
                return Err("builtin configuration cannot also declare widths".into());
            }
            let Object::Name(face) = face else {
                return Err("Builtin must be an exact face name".into());
            };
            let face = BuiltinFace::from_pdf_name(&face).ok_or("unsupported builtin face")?;
            let encoding = data
                .encoding
                .take()
                .ok_or("builtin configuration requires Encoding")?;
            data.builtin = Some(BuiltinMetrics::from_encoding(face, encoding));
        } else {
            let first = byte(spec.get(b"FirstChar".as_slice()))?;
            let last = byte(spec.get(b"LastChar".as_slice()))?;
            let Some(Object::Array(widths)) = spec.get(b"Widths".as_slice()) else {
                return Err("Widths must be a direct array".into());
            };
            if widths.len() > 256 {
                return Err("Widths exceeds 256 entries".into());
            }
            let values = widths.iter().map(number).collect::<Result<Vec<_>, _>>()?;
            let missing = spec
                .get(b"MissingWidth".as_slice())
                .map(number)
                .transpose()?;
            data.declared = Some(
                HorizontalMetrics::from_widths(first, last, &values, missing)
                    .map_err(|e| e.to_string())?,
            );
            if data.unicode.is_none() && data.encoding.is_none() {
                return Err("declared font requires ToUnicode or Encoding".into());
            }
        }
        result.push(data);
    }
    Ok(result)
}
pub(super) fn with_fonts<R>(
    font_path: &str,
    run: impl FnOnce(&TextFonts<'_>) -> Result<R, String>,
) -> Result<R, String> {
    let data = load_fonts(font_path)?;
    let entries = data
        .iter()
        .map(|font| {
            let binding = if let Some(metrics) = &font.builtin {
                TextFont::builtin(metrics, font.unicode.as_ref())
            } else {
                let unicode = if let Some(map) = &font.unicode {
                    UnicodeSource::ToUnicode(map)
                } else {
                    UnicodeSource::Encoding(font.encoding.as_ref().ok_or("missing encoding")?)
                };
                TextFont::declared(unicode, font.declared.as_ref().ok_or("missing metrics")?)
            }
            .map_err(|e| e.to_string())?;
            Ok((font.name.as_slice(), binding))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let fonts = TextFonts::new(&entries, TextFontLimits::default()).map_err(|e| e.to_string())?;
    run(&fonts)
}
pub(super) fn inspect(input: &[u8], font_path: &str) -> Result<Vec<u8>, String> {
    with_fonts(font_path, |fonts| {
        let positioned = position_text_content(
            input,
            fonts,
            TextPositionInitialState::default(),
            ContentLimits::default(),
            TextPositionLimits::default(),
        )
        .map_err(|e| e.to_string())?;
        trace_json(&positioned)
    })
}

fn range(value: std::ops::Range<usize>) -> String {
    format!("[{},{}]", value.start, value.end)
}
fn optional(value: Option<usize>) -> String {
    value
        .map(|n| n.to_string())
        .unwrap_or_else(|| "null".into())
}
fn face_name(face: BuiltinFace) -> &'static str {
    match face {
        BuiltinFace::Helvetica => "Helvetica",
        BuiltinFace::TimesRoman => "Times-Roman",
        BuiltinFace::HelveticaBold => "Helvetica-Bold",
        BuiltinFace::HelveticaBoldOblique => "Helvetica-BoldOblique",
    }
}
fn metric_origin(origin: TextMetricOrigin) -> String {
    match origin {
        TextMetricOrigin::Declared(AdvanceOrigin::WidthsEntry) => "widths".into(),
        TextMetricOrigin::Declared(AdvanceOrigin::DeclaredMissingWidth) => {
            "declared_missing".into()
        }
        TextMetricOrigin::Declared(AdvanceOrigin::DefaultMissingWidth) => "default_missing".into(),
        TextMetricOrigin::Builtin(face) => format!("builtin:{}", face_name(face)),
    }
}
pub(super) fn trace_json(text: &PositionedText) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    append(
        &mut output,
        format!(
            "{{\"text_utf8_hex\":\"{}\",\"source_bytes_hex\":\"{}\",\"fonts\":[",
            super::hex(text.text().as_bytes()),
            super::hex(text.source_bytes())
        )
        .as_bytes(),
    )?;
    for (index, font) in text.fonts().iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        let unicode = match font.unicode_kind() {
            TextUnicodeKind::ToUnicode => "to_unicode",
            TextUnicodeKind::Encoding => "encoding",
        };
        let metrics = match font.metric_kind() {
            TextMetricKind::Declared => "declared".into(),
            TextMetricKind::Builtin(face) => format!("builtin:{}", face_name(face)),
        };
        append(&mut output, format!("{{\"id\":{},\"name_hex\":\"{}\",\"unicode_kind\":\"{unicode}\",\"metric_kind\":\"{metrics}\"}}", font.id().index(), super::hex(font.name())).as_bytes())?;
    }
    append(&mut output, b"],\"codes\":[")?;
    for (index, code) in text.codes().iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        append(&mut output, format!("{{\"code\":{},\"font_id\":{},\"text_object\":{},\"operation_index\":{},\"operation_span\":{},\"operand_index\":{},\"array_index\":{},\"string_span\":{},\"source_span\":{},\"utf8_span\":{},\"width\":{},\"metric_origin\":\"{}\",\"character_spacing\":{},\"word_spacing\":{},\"font_size\":{},\"horizontal_scale\":{},\"rise\":{},\"rendering_mode\":{},\"before\":{:?},\"after\":{:?},\"transform\":{:?},\"origin\":{:?},\"glyph_displacement\":{},\"word_displacement\":{}}}",
            code.code(), code.font_id().index(), code.text_object(), code.operation_index(), range(code.operation_span()), code.operand_index(), optional(code.array_index()), range(code.string_span()), range(code.source_span()), range(code.utf8_span()), code.width(), metric_origin(code.metric_origin()), code.character_spacing(), code.word_spacing(), code.font_size(), code.horizontal_scale(), code.rise(), code.rendering_mode(), code.before().coefficients(), code.after().coefficients(), code.transform().coefficients(), code.origin().coordinates(), code.glyph_displacement(), code.word_displacement()).as_bytes())?;
    }
    append(&mut output, b"],\"adjustments\":[")?;
    for (index, item) in text.adjustments().iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        append(&mut output, format!("{{\"number\":{},\"displacement\":{},\"before\":{:?},\"after\":{:?},\"operation_index\":{},\"operation_span\":{},\"array_index\":{},\"text_object\":{}}}", item.number(), item.displacement(), item.before().coefficients(), item.after().coefficients(), item.operation_index(), range(item.operation_span()), item.array_index(), item.text_object()).as_bytes())?;
    }
    append(&mut output, b"],\"text_objects\":[")?;
    for (index, item) in text.text_objects().iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        append(&mut output, format!("{{\"text_matrix\":{:?},\"line_matrix\":{:?},\"operation_index\":{},\"operation_span\":{}}}", item.text_matrix().coefficients(), item.line_matrix().coefficients(), item.operation_index(), range(item.operation_span())).as_bytes())?;
    }
    append(&mut output, b"],\"events\":[")?;
    for (index, event) in text.events().iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        let (kind, item) = match event {
            TextEvent::Code(n) => ("code", n),
            TextEvent::Adjustment(n) => ("adjustment", n),
            TextEvent::EndTextObject(n) => ("end", n),
        };
        append(&mut output, format!("[\"{kind}\",{item}]").as_bytes())?;
    }
    append(&mut output, b"]}\n")?;
    Ok(output)
}
