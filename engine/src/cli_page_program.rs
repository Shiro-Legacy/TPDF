// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page inspection, with a compact semantic trace for whole-document comparisons.
use super::{append_inspection_output as append, cli_path};
use tandem_pdf_engine::program::{Program, ProgramEventKind};
use tandem_pdf_engine::text_position::TextEvent;

/// Preserve every code and path event in original order without duplicating the
/// much larger text-state/provenance arenas. Full inspection uses C0 trace_json.
fn compact_program(output: &mut Vec<u8>, program: &Program) -> Result<(), String> {
    append(output, b"{\"paths\":")?;
    super::cli_program::paths_json(output, program.paths())?;
    append(output, b",\"text_contexts\":[")?;
    for (id, context) in program.text_contexts().iter().enumerate() {
        if id > 0 {
            append(output, b",")?;
        }
        let clip = context
            .clip()
            .map_or_else(|| "null".into(), |v| v.index().to_string());
        append(
            output,
            format!(
                "{{\"matrix\":{:?},\"clip\":{clip}",
                context.ctm().coefficients()
            )
            .as_bytes(),
        )?;
        cli_path::color(output, context.fill_color())?;
        append(output, b"}")?;
    }
    append(output, b"],\"events\":[")?;
    let mut count = 0;
    for event in program.events() {
        match event.kind() {
            ProgramEventKind::Image(id) => {
                if count > 0 {
                    append(output, b",")?;
                }
                let paint = program.images()[id];
                let clip = paint
                    .clip()
                    .map(|v| v.index().to_string())
                    .unwrap_or_else(|| "null".into());
                append(
                    output,
                    format!(
                        "[\"image\",{},{:?},{clip},{}]",
                        paint.image_id(),
                        paint.ctm().coefficients(),
                        event.operation_index()
                    )
                    .as_bytes(),
                )?;
                count += 1;
            }
            ProgramEventKind::Path(id) => {
                if count > 0 {
                    append(output, b",")?;
                }
                append(
                    output,
                    format!("[\"path\",{id},{}]", event.operation_index()).as_bytes(),
                )?;
                count += 1;
            }
            ProgramEventKind::Text { event: id, context } => {
                let TextEvent::Code(code_id) = program.text().events()[id] else {
                    continue;
                };
                let code = &program.text().codes()[code_id];
                if count > 0 {
                    append(output, b",")?;
                }
                append(output, format!("[\"code\",{},[", code.code()).as_bytes())?;
                for (index, scalar) in program.text().text()[code.utf8_span()].chars().enumerate() {
                    if index > 0 {
                        append(output, b",")?;
                    }
                    append(output, u32::from(scalar).to_string().as_bytes())?;
                }
                append(
                    output,
                    format!(
                        "],{:?},{},{},{context},{},{}]",
                        code.transform().coefficients(),
                        code.width() / 1000.0,
                        code.rendering_mode(),
                        code.operation_index(),
                        code.font_id().index()
                    )
                    .as_bytes(),
                )?;
                count += 1;
            }
        }
    }
    let usage = program.usage();
    append(output, format!("],\"counts\":{{\"codes\":{},\"adjustments\":{},\"text_objects\":{},\"ordered_events\":{}}},\"usage\":{{\"operations\":{},\"child_operations\":{},\"work_units\":{},\"allocated_bytes\":{}}}}}", program.text().codes().len(), program.text().adjustments().len(), program.text().text_objects().len(), program.events().len(), usage.operations, usage.child_operations, usage.work_units, usage.allocated_bytes).as_bytes())
}

use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::font_resources::FontIdentity;
use tandem_pdf_engine::page_program::{
    PageMetricPolicy, PageProgram, PageProgramLimits, PageProgramSession, PageProgramSessionLimits,
};

pub(super) fn string(output: &mut Vec<u8>, value: &str) -> Result<(), String> {
    append(output, b"\"")?;
    for ch in value.chars() {
        match ch {
            '"' => append(output, b"\\\"")?,
            '\\' => append(output, b"\\\\")?,
            '\n' => append(output, b"\\n")?,
            '\r' => append(output, b"\\r")?,
            '\t' => append(output, b"\\t")?,
            ch if ch < '\u{20}' => append(output, format!("\\u{:04x}", ch as u32).as_bytes())?,
            ch => append(output, ch.encode_utf8(&mut [0; 4]).as_bytes())?,
        }
    }
    append(output, b"\"")
}
fn form_object(
    output: &mut Vec<u8>,
    id: Option<tandem_pdf_engine::object::ObjectId>,
) -> Result<(), String> {
    match id {
        Some(id) => append(
            output,
            format!("[{},{}]", id.number(), id.generation()).as_bytes(),
        ),
        None => append(output, b"null"),
    }
}
fn form_origin(
    output: &mut Vec<u8>,
    origin: &tandem_pdf_engine::form_resources::OperationOrigin,
) -> Result<(), String> {
    use tandem_pdf_engine::form_resources::OperationOrigin;
    match origin {
        OperationOrigin::PageContent { span } => append(
            output,
            format!(r#"{{"kind":"page","span":[{},{}]}}"#, span.start, span.end).as_bytes(),
        ),
        OperationOrigin::FormContent { invocation, span } => append(
            output,
            format!(
                r#"{{"kind":"form","invocation":{},"span":[{},{}]}}"#,
                invocation.index(),
                span.start,
                span.end
            )
            .as_bytes(),
        ),
        OperationOrigin::PageScope { phase } => append(
            output,
            format!(
                r#"{{"kind":"page-scope","phase":"{}"}}"#,
                phase_name(*phase)
            )
            .as_bytes(),
        ),
        OperationOrigin::FormBoundary { invocation, phase } => append(
            output,
            format!(
                r#"{{"kind":"boundary","invocation":{},"phase":"{}"}}"#,
                invocation.index(),
                phase_name(*phase)
            )
            .as_bytes(),
        ),
    }
}
fn phase_name(phase: tandem_pdf_engine::form_resources::FormPhase) -> &'static str {
    match phase {
        tandem_pdf_engine::form_resources::FormPhase::Enter => "enter",
        tandem_pdf_engine::form_resources::FormPhase::Leave => "leave",
    }
}
fn form_sources(output: &mut Vec<u8>, page: &PageProgram) -> Result<(), String> {
    append(output, b",\"resource_scopes\":[")?;
    for (index, scope) in page.resource_scopes().iter().enumerate() {
        if index > 0 {
            append(output, b",")?;
        }
        append(
            output,
            format!(r#"{{"id":{},"provider":"#, scope.id().index()).as_bytes(),
        )?;
        form_object(output, scope.provider())?;
        append(output, b",\"resource_object\":")?;
        form_object(output, scope.resource_object())?;
        append(output, b"}")?;
    }
    append(output, b"],\"form_streams\":[")?;
    for (index, stream) in page.form_streams().iter().enumerate() {
        if index > 0 {
            append(output, b",")?;
        }
        append(output, format!(r#"{{"id":{index},"object":"#).as_bytes())?;
        form_object(output, Some(stream.object_id()))?;
        append(output, b",\"content_hex\":\"")?;
        for chunk in stream.bytes().chunks(4096) {
            append(output, super::hex(chunk).as_bytes())?;
        }
        append(output, b"\"}")?;
    }
    append(output, b"],\"form_invocations\":[")?;
    for (index, invocation) in page.form_invocations().iter().enumerate() {
        if index > 0 {
            append(output, b",")?;
        }
        let parent = invocation
            .parent()
            .map_or_else(|| "null".into(), |id| id.index().to_string());
        let range = invocation.execution_range();
        append(
            output,
            format!(
                r#"{{"id":{},"parent":{parent},"stream_id":{},"object":"#,
                invocation.id().index(),
                invocation.stream_id().index()
            )
            .as_bytes(),
        )?;
        form_object(output, Some(invocation.object_id()))?;
        append(output,format!(r#", "caller_scope":{},"effective_scope":{},"matrix":{:?},"bbox":{:?},"execution_range":[{},{}],"call_origin":"#,invocation.caller_scope().index(),invocation.effective_scope().index(),invocation.matrix().coefficients(),invocation.bbox().bounds(),range.start,range.end).as_bytes())?;
        form_origin(output, invocation.call_origin())?;
        append(output, b"}")?;
    }
    append(output, b"],\"operation_origins\":[")?;
    for index in 0..page.program().usage().operations {
        if index > 0 {
            append(output, b",")?;
        }
        let origin = page
            .operation_origin(index)
            .ok_or("missing execution source origin")?;
        form_origin(output, origin)?;
    }
    append(output, b"]")
}

pub(super) fn page_json(
    output: &mut Vec<u8>,
    page: &PageProgram,
    compact: bool,
) -> Result<(), String> {
    append(output, format!("{{\"page\":{},\"status\":\"accepted\",\"object\":[{},{}],\"metric_policy\":\"AllowPinnedBase14\",\"geometry\":{{\"bounds\":{:?},\"user_to_page\":{:?}}},\"content_bytes\":{},\"segments\":[", page.page_index()+1, page.page_id().number(), page.page_id().generation(), page.geometry().bounds().bounds(), page.geometry().user_to_page().coefficients(), page.content().bytes.len()).as_bytes())?;
    for (index, segment) in page.content().segments.iter().enumerate() {
        if index > 0 {
            append(output, b",")?;
        }
        let padding = segment
            .padding
            .map_or_else(|| "null".into(), |n| n.to_string());
        append(
            output,
            format!(
                "{{\"object\":[{},{}],\"range\":[{},{}],\"padding\":{padding}}}",
                segment.object.number(),
                segment.object.generation(),
                segment.range.start,
                segment.range.end
            )
            .as_bytes(),
        )?;
    }
    append(output, b"],\"fonts\":[")?;
    for (index, font) in page.fonts().iter().enumerate() {
        if index > 0 {
            append(output, b",")?;
        }
        append(
            output,
            format!(
                "{{\"id\":{index},\"name_hex\":\"{}\",\"identity\":",
                super::hex(font.name())
            )
            .as_bytes(),
        )?;
        match font.identity() {
            FontIdentity::Indirect(id) => append(
                output,
                format!("{{\"indirect\":[{},{}]}}", id.number(), id.generation()).as_bytes(),
            )?,
            FontIdentity::Direct {
                resource_owner,
                resource_index,
            } => append(
                output,
                format!(
                    "{{\"resource_owner\":[{},{}],\"resource_index\":{resource_index}}}",
                    resource_owner.number(),
                    resource_owner.generation()
                )
                .as_bytes(),
            )?,
        }
        append(
            output,
            format!(",\"scope_id\":{}", font.scope_id().index()).as_bytes(),
        )?;
        append(output, b",\"metric_kind\":")?;
        string(output, &format!("{:?}", font.metric_kind()))?;
        append(output, b",\"unicode_kind\":")?;
        string(output, &format!("{:?}", font.unicode_kind()))?;
        append(
            output,
            format!(",\"first_operation\":{}}}", font.first_operation()).as_bytes(),
        )?;
    }
    append(output, b"]")?;
    if !compact {
        append(output, b",\"content_hex\":\"")?;
        // Hex in bounded chunks avoids doubling the entire joined buffer first.
        for chunk in page.content().bytes.chunks(4096) {
            append(output, super::hex(chunk).as_bytes())?;
        }
        append(output, b"\"")?;
    }
    form_sources(output, page)?;
    append(output, b",\"program\":")?;
    if compact {
        compact_program(output, page.program())?;
    } else {
        append(output, &super::cli_program::trace_json(page.program())?)?;
    }
    append(output, b"}")
}
fn usage_json(output: &mut Vec<u8>, session: &PageProgramSession<'_, '_>) -> Result<(), String> {
    let u = session.usage();
    append(output,format!("{{\"page_attempts\":{},\"content_references\":{},\"encoded_bytes\":{},\"decoded_work_bytes\":{},\"joined_bytes\":{},\"parser_input_bytes\":{},\"parser_nodes\":{},\"parser_owned_bytes\":{},\"parser_tokens\":{},\"parser_operations\":{},\"prepared_fonts\":{},\"font_name_bytes\":{},\"program_operations\":{},\"program_work_units\":{},\"allocated_bytes\":{}}}",u.page_attempts,u.content_references,u.encoded_bytes,u.decoded_work_bytes,u.joined_bytes,u.parser_input_bytes,u.parser_nodes,u.parser_owned_bytes,u.parser_tokens,u.parser_operations,u.prepared_fonts,u.font_name_bytes,u.program_operations,u.program_work_units,u.allocated_bytes).as_bytes())
}
pub(super) fn inspect(input: &[u8], selected: Option<usize>) -> Result<Vec<u8>, String> {
    let document = Document::open(input, DocumentLimits::default()).map_err(|e| e.to_string())?;
    let mut session = PageProgramSession::new(
        &document,
        PageProgramSessionLimits {
            metric_policy: PageMetricPolicy::AllowPinnedBase14,
            ..PageProgramSessionLimits::default()
        },
    )
    .map_err(|e| e.to_string())?;
    if selected.is_some_and(|index| index >= session.page_count()) {
        return Err("page-program index is outside the document".into());
    }
    let mut output = Vec::new();
    append(&mut output,b"{\"schema\":1,\"metric_policy\":\"AllowPinnedBase14\",\"scope\":\"mixed path and positioned source-code traces; no glyph or image rendering\",\"prior_document_work\":\"separately bounded by DocumentLimits\",\"pages\":[")?;
    let indices = selected.map_or(0..session.page_count(), |n| n..n + 1);
    for (count, index) in indices.enumerate() {
        if count > 0 {
            append(&mut output, b",")?;
        }
        match session.trace_page(index, PageProgramLimits::default()) {
            Ok(page) => page_json(&mut output, &page, selected.is_none())?,
            Err(error) => {
                // A single-page request fails atomically. Corpus mode records each
                // refusal and keeps using the same cumulative session budget.
                if selected.is_some() {
                    return Err(error.to_string());
                }
                append(
                    &mut output,
                    format!("{{\"page\":{},\"status\":\"refused\",\"error\":", index + 1)
                        .as_bytes(),
                )?;
                string(&mut output, &error.to_string())?;
                let operation = error
                    .operation_index
                    .map_or_else(|| "null".into(), |v| v.to_string());
                let span = error
                    .span
                    .as_ref()
                    .map_or_else(|| "null".into(), |s| format!("[{},{}]", s.start, s.end));
                append(
                    &mut output,
                    format!(",\"operation_index\":{operation},\"span\":{span},\"source_object\":")
                        .as_bytes(),
                )?;
                form_object(&mut output, error.source_object)?;
                append(&mut output, b",\"diagnostic\":[")?;
                for (n, frame) in error.diagnostic.iter().enumerate() {
                    if n > 0 {
                        append(&mut output, b",")?;
                    }
                    append(&mut output, b"{\"object\":")?;
                    form_object(&mut output, Some(frame.object))?;
                    append(
                        &mut output,
                        format!(
                            ",\"span\":[{},{}],\"caller_object\":",
                            frame.span.start, frame.span.end
                        )
                        .as_bytes(),
                    )?;
                    form_object(&mut output, Some(frame.caller_object))?;
                    append(
                        &mut output,
                        format!(
                            ",\"call_span\":[{},{}]}}",
                            frame.call_span.start, frame.call_span.end
                        )
                        .as_bytes(),
                    )?;
                }
                append(&mut output, b"]}")?;
            }
        }
    }
    append(&mut output, b"],\"session_usage\":")?;
    usage_json(&mut output, &session)?;
    append(&mut output, b",\"form_usage\":{")?;
    let form_usage = session.form_usage();
    append(
        &mut output,
        format!("\"invocations\":{}", form_usage.invocations).as_bytes(),
    )?;
    append(
        &mut output,
        format!(",\"unique_streams\":{}", form_usage.unique_streams).as_bytes(),
    )?;
    append(
        &mut output,
        format!(",\"resource_scopes\":{}", form_usage.resource_scopes).as_bytes(),
    )?;
    append(
        &mut output,
        format!(",\"resource_bindings\":{}", form_usage.resource_bindings).as_bytes(),
    )?;
    append(
        &mut output,
        format!(",\"name_bytes\":{}", form_usage.name_bytes).as_bytes(),
    )?;
    append(
        &mut output,
        format!(",\"metadata_items\":{}", form_usage.metadata_items).as_bytes(),
    )?;
    append(
        &mut output,
        format!(",\"reference_hops\":{}", form_usage.reference_hops).as_bytes(),
    )?;
    append(
        &mut output,
        format!(",\"lookup_work\":{}", form_usage.lookup_work).as_bytes(),
    )?;
    append(
        &mut output,
        format!(",\"origin_records\":{}", form_usage.origin_records).as_bytes(),
    )?;
    append(
        &mut output,
        format!(",\"allocated_bytes\":{}", form_usage.allocated_bytes).as_bytes(),
    )?;
    append(
        &mut output,
        format!(
            ",\"diagnostic_allocated_bytes\":{}",
            form_usage.diagnostic_allocated_bytes
        )
        .as_bytes(),
    )?;
    append(&mut output, b"}")?;
    let f = session.font_usage();
    append(&mut output,format!(",\"font_usage\":{{\"calls\":{},\"fonts\":{},\"maps\":{},\"metrics\":{},\"builtin_metrics\":{},\"encoded_bytes\":{},\"decoded_work_bytes\":{},\"reference_hops\":{},\"metadata_items\":{},\"bindings\":{},\"cmap_entries\":{},\"cmap_payload_bytes\":{}}}}}\n",f.calls,f.fonts,f.maps,f.metrics,f.builtin_metrics,f.encoded_bytes,f.decoded_work_bytes,f.reference_hops,f.metadata_items,f.bindings,f.cmap_entries,f.cmap_payload_bytes).as_bytes())?;
    Ok(output)
}
