// SPDX-License-Identifier: AGPL-3.0-or-later
use super::append_inspection_output as append;
use std::collections::BTreeSet;
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::font_resources::{
    FontIdentity, FontResourceErrorKind, FontResourceLimits, FontResourceLoader, SimpleFontMapping,
    SimpleFontSubtype,
};
use tandem_pdf_engine::object::ObjectId;
use tandem_pdf_engine::pages::{PageLimits, collect_pages};
fn identity(value: FontIdentity) -> String {
    match value {
        FontIdentity::Indirect(id) => format!("i:{}:{}", id.number(), id.generation()),
        FontIdentity::Direct {
            resource_owner,
            resource_index,
        } => format!(
            "d:{}:{}:{}",
            resource_owner.number(),
            resource_owner.generation(),
            resource_index
        ),
    }
}
fn object(value: Option<ObjectId>) -> String {
    value
        .map(|id| format!("[{},{}]", id.number(), id.generation()))
        .unwrap_or_else(|| "null".into())
}
pub(super) fn inspect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let document = Document::open(bytes, DocumentLimits::default()).map_err(|e| e.to_string())?;
    let pages = collect_pages(&document, PageLimits::default()).map_err(|e| e.to_string())?;
    let mut loader = FontResourceLoader::new(&document, FontResourceLimits::default())
        .map_err(|e| e.to_string())?;
    let mut output = Vec::new();
    let mut fonts = BTreeSet::new();
    append(&mut output, b"{\"pages\":[")?;
    for (page_index, page) in pages.iter().enumerate() {
        if page_index > 0 {
            append(&mut output, b",")?;
        }
        let bindings = loader.page_bindings(page.id).map_err(|e| e.to_string())?;
        append(
            &mut output,
            format!(
                "{{\"page\":{},\"object\":{},\"resource_owner\":{},\"bindings\":[",
                page_index + 1,
                object(Some(page.id)),
                object(bindings.resource_owner())
            )
            .as_bytes(),
        )?;
        // Names are bounded by the loader; copying this page's names permits
        // further mutable cache loads without self-referential borrows.
        let names: Vec<_> = bindings
            .names()
            .map(|name| (name.to_vec(), bindings.target(name).flatten()))
            .collect();
        for (index, (name, target)) in names.iter().enumerate() {
            if index > 0 {
                append(&mut output, b",")?;
            }
            append(
                &mut output,
                format!(
                    "{{\"name_hex\":\"{}\",\"target\":{},",
                    super::hex(name),
                    object(*target)
                )
                .as_bytes(),
            )?;
            match loader.load(page.id, name) {
                Ok(font) => {
                    fonts.insert(font.identity());
                    append(
                        &mut output,
                        format!(
                            "\"status\":\"loaded\",\"font\":\"{}\"}}",
                            identity(font.identity())
                        )
                        .as_bytes(),
                    )?;
                }
                Err(error) => {
                    let status = match error.kind {
                        FontResourceErrorKind::Unsupported(_) => "unsupported",
                        _ => "error",
                    };
                    append(
                        &mut output,
                        format!(
                            "\"status\":\"{status}\",\"diagnostic_hex\":\"{}\"}}",
                            super::hex(error.to_string().as_bytes())
                        )
                        .as_bytes(),
                    )?;
                }
            }
        }
        append(&mut output, b"]}")?;
    }
    append(&mut output, b"],\"fonts\":[")?;
    for (index, id) in fonts.into_iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        let font = loader
            .loaded(id)
            .ok_or("font cache lost a successful identity")?;
        let subtype = match font.subtype() {
            SimpleFontSubtype::Type1 => "Type1",
            SimpleFontSubtype::TrueType => "TrueType",
            SimpleFontSubtype::Type0 => "Type0",
        };
        let base = font
            .base_font()
            .map(|v| format!("\"{}\"", super::hex(v)))
            .unwrap_or_else(|| "null".into());
        append(
            &mut output,
            format!(
                "{{\"id\":\"{}\",\"subtype\":\"{subtype}\",\"base_font_hex\":{base},\"mapping\":{{",
                identity(id)
            )
            .as_bytes(),
        )?;
        match font.mapping() {
            SimpleFontMapping::ExplicitEncoding { base, encoding } => {
                let name = std::str::from_utf8(base.pdf_name())
                    .map_err(|_| "encoding constant is not ASCII")?;
                append(
                    &mut output,
                    format!("\"kind\":\"explicit_encoding\",\"base\":\"{name}\",\"slots\":")
                        .as_bytes(),
                )?;
                super::cli_encoding::write_slots(&mut output, encoding)?;
            }
            SimpleFontMapping::ToUnicode(map) => {
                append(&mut output, b"\"kind\":\"to_unicode\",\"codespaces\":[")?;
                for (index, space) in map.codespaces().iter().enumerate() {
                    if index > 0 {
                        append(&mut output, b",")?;
                    }
                    append(
                        &mut output,
                        format!("[{},{}]", space.low().value(), space.high().value()).as_bytes(),
                    )?;
                }
                append(&mut output, b"],\"entries\":[")?;
                for (index, (code, sequence)) in map.mappings().enumerate() {
                    if index > 0 {
                        append(&mut output, b",")?;
                    }
                    append(
                        &mut output,
                        format!("{{\"code\":{},\"scalars\":[", code.value()).as_bytes(),
                    )?;
                    for (i, scalar) in sequence.scalars().iter().enumerate() {
                        if i > 0 {
                            append(&mut output, b",")?;
                        }
                        append(&mut output, (*scalar as u32).to_string().as_bytes())?;
                    }
                    append(&mut output, b"]}")?;
                }
                append(&mut output, b"]")?;
            }
        }
        append(&mut output, b"}}")?;
    }
    let usage = loader.usage();
    append(&mut output,format!("],\"usage\":{{\"calls\":{},\"reference_hops\":{},\"fonts\":{},\"maps\":{},\"bindings\":{},\"encoded_bytes\":{},\"decoded_work_bytes\":{}}}}}\n",usage.calls,usage.reference_hops,usage.fonts,usage.maps,usage.bindings,usage.encoded_bytes,usage.decoded_work_bytes).as_bytes())?;
    Ok(output)
}

fn advance_origin(origin: tandem_pdf_engine::font_metrics::AdvanceOrigin) -> &'static str {
    use tandem_pdf_engine::font_metrics::AdvanceOrigin;
    match origin {
        AdvanceOrigin::WidthsEntry => "widths",
        AdvanceOrigin::DeclaredMissingWidth => "declared_missing",
        AdvanceOrigin::DefaultMissingWidth => "default_missing",
    }
}
pub(super) fn inspect_metrics(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let document = Document::open(bytes, DocumentLimits::default()).map_err(|e| e.to_string())?;
    let pages = collect_pages(&document, PageLimits::default()).map_err(|e| e.to_string())?;
    let mut loader = FontResourceLoader::new(&document, FontResourceLimits::default())
        .map_err(|e| e.to_string())?;
    let mut output = Vec::new();
    let mut fonts = BTreeSet::new();
    append(&mut output, b"{\"pages\":[")?;
    for (index, page) in pages.iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        let bindings = loader.page_bindings(page.id).map_err(|e| e.to_string())?;
        let names: Vec<_> = bindings
            .names()
            .map(|name| (name.to_vec(), bindings.target(name).flatten()))
            .collect();
        append(
            &mut output,
            format!("{{\"page\":{},\"bindings\":[", index + 1).as_bytes(),
        )?;
        for (i, (name, target)) in names.iter().enumerate() {
            if i > 0 {
                append(&mut output, b",")?;
            }
            append(
                &mut output,
                format!(
                    "{{\"name_hex\":\"{}\",\"target\":{},",
                    super::hex(name),
                    object(*target)
                )
                .as_bytes(),
            )?;
            match loader.load_metrics(page.id, name) {
                Ok(font) => {
                    fonts.insert(font.identity());
                    append(
                        &mut output,
                        format!(
                            "\"status\":\"loaded\",\"font\":\"{}\"}}",
                            identity(font.identity())
                        )
                        .as_bytes(),
                    )?;
                }
                Err(error) => {
                    let status = match error.kind {
                        FontResourceErrorKind::NeedsBuiltinOrFontProgramMetrics => {
                            "needs_builtin_metrics"
                        }
                        FontResourceErrorKind::Unsupported(_) => "unsupported",
                        _ => "error",
                    };
                    append(
                        &mut output,
                        format!(
                            "\"status\":\"{status}\",\"diagnostic_hex\":\"{}\"}}",
                            super::hex(error.to_string().as_bytes())
                        )
                        .as_bytes(),
                    )?;
                }
            }
        }
        append(&mut output, b"]}")?;
    }
    append(&mut output, b"],\"fonts\":[")?;
    for (index, id) in fonts.into_iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        let metrics = loader
            .loaded_metrics(id)
            .ok_or("metrics cache lost identity")?
            .metrics();
        let missing = metrics.missing();
        append(&mut output,format!("{{\"id\":\"{}\",\"first\":{},\"last\":{},\"missing\":{{\"advance\":{},\"origin\":\"{}\"}},\"lookups\":[",identity(id),metrics.first(),metrics.last(),missing.advance().value(),advance_origin(missing.origin())).as_bytes())?;
        for code in 0..=u8::MAX {
            if code > 0 {
                append(&mut output, b",")?;
            }
            let value = metrics.lookup(code);
            append(
                &mut output,
                format!(
                    "{{\"code\":{code},\"advance\":{},\"origin\":\"{}\"}}",
                    value.advance().value(),
                    advance_origin(value.origin())
                )
                .as_bytes(),
            )?;
        }
        append(&mut output, b"]}")?;
    }
    let usage = loader.usage();
    append(&mut output,format!("],\"usage\":{{\"metric_attempts\":{},\"metrics\":{},\"width_entries\":{},\"retained_width_entries\":{}}}}}\n",usage.metric_attempts,usage.metrics,usage.width_entries,usage.retained_width_entries).as_bytes())?;
    Ok(output)
}

pub(super) fn inspect_builtin_metrics(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let document = Document::open(bytes, DocumentLimits::default()).map_err(|e| e.to_string())?;
    let pages = collect_pages(&document, PageLimits::default()).map_err(|e| e.to_string())?;
    let mut loader = FontResourceLoader::new(&document, FontResourceLimits::default())
        .map_err(|e| e.to_string())?;
    let mut output = Vec::new();
    let mut fonts = BTreeSet::new();
    append(&mut output, b"{\"pages\":[")?;
    for (index, page) in pages.iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        let bindings = loader.page_bindings(page.id).map_err(|e| e.to_string())?;
        let names: Vec<_> = bindings
            .names()
            .map(|name| (name.to_vec(), bindings.target(name).flatten()))
            .collect();
        append(
            &mut output,
            format!("{{\"page\":{},\"bindings\":[", index + 1).as_bytes(),
        )?;
        for (i, (name, target)) in names.iter().enumerate() {
            if i > 0 {
                append(&mut output, b",")?;
            }
            append(
                &mut output,
                format!(
                    "{{\"name_hex\":\"{}\",\"target\":{},",
                    super::hex(name),
                    object(*target)
                )
                .as_bytes(),
            )?;
            match loader.load_builtin_metrics(page.id, name) {
                Ok(font) => {
                    fonts.insert(font.identity());
                    append(
                        &mut output,
                        format!(
                            "\"status\":\"loaded\",\"font\":\"{}\"}}",
                            identity(font.identity())
                        )
                        .as_bytes(),
                    )?;
                }
                Err(error) => {
                    let status = match error.kind {
                        FontResourceErrorKind::NeedsBuiltinOrFontProgramMetrics => {
                            "needs_builtin_metrics"
                        }
                        FontResourceErrorKind::Unsupported(_) => "unsupported",
                        _ => "error",
                    };
                    append(
                        &mut output,
                        format!(
                            "\"status\":\"{status}\",\"diagnostic_hex\":\"{}\"}}",
                            super::hex(error.to_string().as_bytes())
                        )
                        .as_bytes(),
                    )?;
                }
            }
        }
        append(&mut output, b"]}")?;
    }
    append(&mut output, b"],\"fonts\":[")?;
    for (index, id) in fonts.into_iter().enumerate() {
        if index > 0 {
            append(&mut output, b",")?;
        }
        let metrics = loader
            .loaded_builtin_metrics(id)
            .ok_or("metrics cache lost identity")?
            .metrics();
        append(
            &mut output,
            format!(
                "{{\"id\":\"{}\",\"face\":\"{}\",\"slots\":",
                identity(id),
                std::str::from_utf8(metrics.face().pdf_name())
                    .map_err(|_| "non-ASCII face constant")?
            )
            .as_bytes(),
        )?;
        super::cli_builtin::write_slots(&mut output, metrics)?;
        append(&mut output, b"}")?;
    }
    let usage = loader.usage();
    append(&mut output,format!("],\"usage\":{{\"builtin_metric_attempts\":{},\"builtin_metrics\":{},\"builtin_slots\":{},\"retained_builtin_slots\":{}}}}}\n",usage.builtin_metric_attempts,usage.builtin_metrics,usage.builtin_slots,usage.retained_builtin_slots).as_bytes())?;
    Ok(output)
}
