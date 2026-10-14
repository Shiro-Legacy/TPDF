// SPDX-License-Identifier: AGPL-3.0-or-later
use super::{
    append_inspection_output as append,
    cli_page_program::{page_json, string},
};
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    embedded_glyph::GlyphRoute,
    page_glyph::{
        PageGlyphCommand, PageGlyphLimits, PageGlyphProgram, PageGlyphSession,
        PageGlyphSessionLimits, PageOutlinePolicy,
    },
    page_program::PageMetricPolicy,
};
fn page(out: &mut Vec<u8>, p: &PageGlyphProgram, include_source_kind: bool) -> Result<(), String> {
    append(
        out,
        format!(
            "{{\"page\":{},\"status\":\"accepted\",\"trace\":",
            p.page_program().page_index() + 1
        )
        .as_bytes(),
    )?;
    page_json(out, p.page_program(), false)?;
    append(out, b",\"glyphs\":[")?;
    for (i, g) in p.glyphs().iter().enumerate() {
        if i > 0 {
            append(out, b",")?;
        }
        let r = g.command_range();
        if let Some(t) = g.embedded_true_type() {
            let c = t.choice();
            let route = match c.map(|c| c.route()) {
                None => "cid",
                Some(GlyphRoute::RawByte) => "raw_byte",
                Some(GlyphRoute::PrivateUse) => "private_use",
                Some(GlyphRoute::EncodingUnicode) => "encoding_unicode",
                Some(GlyphRoute::EncodingMacRoman) => "encoding_mac_roman",
                Some(GlyphRoute::Missing) => "missing",
                Some(GlyphRoute::UnsupportedNameFallback) => "name_fallback",
            };
            let gid = t.glyph_id();
            let lookup = c
                .and_then(|c| c.lookup_code())
                .map_or_else(|| "null".into(), |n| n.to_string());
            let source = c.map_or_else(
                || p.page_program().program().text().codes()[g.code_index()].code(),
                |c| u32::from(c.source()),
            );
            let selection = t
                .selection()
                .map_or_else(|| "null".into(), super::cli_glyph::selection);
            let h = t.hhea_range();
            let m = t.hmtx_range();
            append(
                out,
                if include_source_kind {
                    b"{\"source_kind\":\"embedded_true_type\","
                } else {
                    b"{"
                },
            )?;
            append(out,format!("\"code_index\":{},\"event_index\":{},\"context\":{},\"font\":[{},{}],\"program\":[{},{}],\"source\":{},\"gid\":{},\"route\":\"{}\",\"lookup_code\":{},\"units_per_em\":{},\"origin_shift\":{},\"metric_lsb\":{},\"hhea_range\":[{},{}],\"hmtx_range\":[{},{}],\"command_range\":[{},{}],\"selection\":{}}}",g.code_index(),g.event_index(),g.context_id(),g.font_id().number(),g.font_id().generation(),t.program_id().number(),t.program_id().generation(),source,gid,route,lookup,g.units_per_em(),t.origin_shift(),t.metric_lsb(),h.start,h.end,m.start,m.end,r.start,r.end,selection).as_bytes())?;
        } else if let Some(asset) = g.pinned_base14() {
            let code = p.page_program().program().text().codes()[g.code_index()].code();
            append(out,format!("{{\"source_kind\":\"pinned_base14\",\"code_index\":{},\"event_index\":{},\"context\":{},\"font\":[{},{}],\"source\":{},\"native_gid\":{},\"units_per_em\":{},\"asset_sha256\":\"{}\",\"command_range\":[{},{}],\"face\":",g.code_index(),g.event_index(),g.context_id(),g.font_id().number(),g.font_id().generation(),code,asset.native_gid(),asset.units_per_em(),asset.asset_sha256(),r.start,r.end).as_bytes())?;
            string(
                out,
                std::str::from_utf8(asset.face().pdf_name())
                    .map_err(|_| "invalid pinned face name")?,
            )?;
            append(out, b",\"glyph_name\":")?;
            string(
                out,
                std::str::from_utf8(asset.name()).map_err(|_| "invalid pinned glyph name")?,
            )?;
            append(out, b"}")?;
        } else {
            return Err("page glyph source invariant".into());
        }
    }
    append(out, b"],\"commands\":[")?;
    for (i, c) in p.commands().iter().enumerate() {
        if i > 0 {
            append(out, b",")?;
        }
        let s = match c {
            PageGlyphCommand::MoveTo(p) => format!("[\"m\",{},{}]", p.x(), p.y()),
            PageGlyphCommand::LineTo(p) => format!("[\"l\",{},{}]", p.x(), p.y()),
            PageGlyphCommand::QuadTo { control, end } => format!(
                "[\"q\",{},{},{},{}]",
                control.x(),
                control.y(),
                end.x(),
                end.y()
            ),
            PageGlyphCommand::CubicTo {
                control1,
                control2,
                end,
            } => format!(
                "[\"c\",{},{},{},{},{},{}]",
                control1.x(),
                control1.y(),
                control2.x(),
                control2.y(),
                end.x(),
                end.y()
            ),
            PageGlyphCommand::Close => "[\"h\"]".into(),
        };
        append(out, s.as_bytes())?;
    }
    append(out, b"]}")
}
pub(super) fn inspect(input: &[u8], selected: Option<usize>) -> Result<Vec<u8>, String> {
    inspect_policy(input, selected, PageOutlinePolicy::EmbeddedOnly)
}
pub(super) fn inspect_policy(
    input: &[u8],
    selected: Option<usize>,
    policy: PageOutlinePolicy,
) -> Result<Vec<u8>, String> {
    let doc = Document::open(input, DocumentLimits::default()).map_err(|e| e.to_string())?;
    let mut limits = PageGlyphSessionLimits::default();
    limits.page.metric_policy = PageMetricPolicy::AllowPinnedBase14;
    limits.outline_policy = policy;
    let mut session = PageGlyphSession::new(&doc, limits).map_err(|e| e.to_string())?;
    let indices = if let Some(i) = selected {
        if i >= session.page_count() {
            return Err("page index outside document".into());
        }
        i..i + 1
    } else {
        0..session.page_count()
    };
    let mut out = Vec::new();
    append(&mut out, b"{\"pages\":[")?;
    for (count, index) in indices.enumerate() {
        if count > 0 {
            append(&mut out, b",")?;
        }
        match session.trace_page(index, PageGlyphLimits::default()) {
            Ok(p) => page(&mut out, &p, policy == PageOutlinePolicy::AllowPinnedBase14)?,
            Err(e) => {
                if selected.is_some() {
                    return Err(e.to_string());
                }
                append(
                    &mut out,
                    format!("{{\"page\":{},\"status\":\"refused\",\"error\":", index + 1)
                        .as_bytes(),
                )?;
                string(&mut out, &e.to_string())?;
                let c = e
                    .code_index
                    .map_or_else(|| "null".into(), |n| n.to_string());
                let op = e
                    .operation_index
                    .map_or_else(|| "null".into(), |n| n.to_string());
                append(
                    &mut out,
                    format!(",\"code_index\":{c},\"operation_index\":{op}}}").as_bytes(),
                )?;
            }
        }
    }
    let u = session.usage();
    let f = u.font;
    let c = u.cmap;
    let o = u.outline;
    let l = session.load_usage();
    let p = session.page_usage();
    append(&mut out,format!("],\"usage\":{{\"cached_fonts\":{},\"cached_glyphs\":{},\"occurrences\":{},\"transformed_commands\":{},\"transform_points\":{},\"allocated_bytes\":{},\"metrics_work\":{},\"lookup_work\":{},\"font\":{{\"index_allocation_bytes\":{},\"work\":{}}},\"cmap\":{{\"directory_records\":{},\"selection_attempts\":{},\"segments\":{},\"validated_codes\":{},\"queries\":{},\"work\":{}}},\"outline\":{{\"decode_attempts\":{},\"glyph_bytes\":{},\"contours\":{},\"points\":{},\"commands\":{},\"allocation_bytes\":{},\"work\":{}}},\"load\":{{\"attempts\":{},\"reference_hops\":{},\"metadata_items\":{},\"encoded_bytes\":{},\"decoded_work_bytes\":{},\"name_bytes\":{}}},\"page\":{{\"page_attempts\":{},\"program_operations\":{},\"program_work_units\":{},\"allocated_bytes\":{}}}}}}}\n",u.cached_fonts,u.cached_glyphs,u.occurrences,u.transformed_commands,u.transform_points,u.allocated_bytes,u.metrics_work,u.lookup_work,f.index_allocation_bytes,f.work,c.directory_records,c.selection_attempts,c.segments,c.validated_codes,c.queries,c.work,o.decode_attempts,o.glyph_bytes,o.contours,o.points,o.commands,o.allocation_bytes,o.work,l.attempts,l.reference_hops,l.metadata_items,l.encoded_bytes,l.decoded_work_bytes,l.name_bytes,p.page_attempts,p.program_operations,p.program_work_units,p.allocated_bytes).as_bytes())?;
    Ok(out)
}
