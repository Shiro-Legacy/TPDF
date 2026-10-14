// SPDX-License-Identifier: AGPL-3.0-or-later
use super::append_inspection_output as append;
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    embedded_glyph::{GlyphFontLoader, GlyphLoadLimits, GlyphRoute},
    object::ObjectId,
    truetype_cmap::{CmapLimits, CmapLookup, CmapSelection, TrueTypeCmapSession},
    truetype_outline::{FontLimits, OutlineLimits, TrueTypeFont},
};
pub(super) fn selection(s: &CmapSelection) -> String {
    let r = s.record();
    let c = s.cmap_range();
    let t = s.subtable_range();
    format!(
        "{{\"index\":{},\"platform\":{},\"encoding\":{},\"format\":{},\"language\":{},\"segments\":{},\"cmap_range\":[{},{}],\"subtable_range\":[{},{}]}}",
        r.index(),
        r.platform_id(),
        r.encoding_id(),
        r.format(),
        s.language(),
        s.segment_count(),
        c.start,
        c.end,
        t.start,
        t.end
    )
}
pub(super) fn cmap(bytes: &[u8], selector: &str) -> Result<Vec<u8>, String> {
    let font = TrueTypeFont::open(bytes, FontLimits::default()).map_err(|e| e.to_string())?;
    let mut session =
        TrueTypeCmapSession::new(&font, CmapLimits::default()).map_err(|e| e.to_string())?;
    let indices: Vec<usize> = if selector == "all" {
        session.records().map(|r| r.index()).collect()
    } else {
        vec![
            selector
                .parse()
                .map_err(|_| "cmap selector must be all or a record index")?,
        ]
    };
    let mut out = Vec::new();
    append(
        &mut out,
        format!("{{\"glyph_count\":{},\"records\":[", font.glyph_count()).as_bytes(),
    )?;
    for (i, index) in indices.into_iter().enumerate() {
        if i > 0 {
            append(&mut out, b",")?;
        }
        let selected = session.select(index).map_err(|e| e.to_string())?;
        append(
            &mut out,
            format!("{{\"selection\":{},\"gids\":[", selection(&selected)).as_bytes(),
        )?;
        for code in 0..=u16::MAX {
            if code > 0 {
                append(&mut out, b",")?;
            }
            let gid = match session.lookup(code).map_err(|e| e.to_string())? {
                CmapLookup::Missing => 0,
                CmapLookup::Glyph(id) => id.get(),
            };
            append(&mut out, gid.to_string().as_bytes())?;
        }
        append(&mut out, b"]}")?;
    }
    let u = session.usage();
    append(&mut out,format!("],\"usage\":{{\"records\":{},\"selections\":{},\"segments\":{},\"validated_codes\":{},\"queries\":{},\"work\":{}}}}}\n",u.directory_records,u.selection_attempts,u.segments,u.validated_codes,u.queries,u.work).as_bytes())?;
    Ok(out)
}
pub(super) fn font(bytes: &[u8], selector: &str) -> Result<Vec<u8>, String> {
    let (n, g) = selector
        .split_once(':')
        .ok_or("font selector must be N:G")?;
    let id = ObjectId::new(
        n.parse().map_err(|_| "font object number")?,
        g.parse().map_err(|_| "font generation")?,
    )
    .map_err(|e| e.to_string())?;
    let doc = Document::open(bytes, DocumentLimits::default()).map_err(|e| e.to_string())?;
    let mut loader = GlyphFontLoader::new(&doc, GlyphLoadLimits::default());
    let font = loader.load(id).map_err(|e| e.to_string())?;
    let outlines = font
        .outlines(FontLimits::default(), CmapLimits::default())
        .map_err(|e| e.to_string())?;
    let mut session = outlines.session(OutlineLimits::default());
    let mut out = Vec::new();
    append(&mut out,format!("{{\"font\":[{},{}],\"program\":[{},{}],\"flags\":{},\"units_per_em\":{},\"glyph_count\":{},\"selection\":{},\"slots\":[",id.number(),id.generation(),font.program_id().number(),font.program_id().generation(),font.flags(),outlines.units_per_em(),outlines.glyph_count(),selection(outlines.map().selection())).as_bytes())?;
    for choice in outlines.map().choices() {
        if choice.source() > 0 {
            append(&mut out, b",")?;
        }
        let (status, route) = match choice.route() {
            GlyphRoute::RawByte => ("mapped", "raw_byte"),
            GlyphRoute::PrivateUse => ("mapped", "private_use"),
            GlyphRoute::EncodingUnicode => ("mapped", "encoding_unicode"),
            GlyphRoute::EncodingMacRoman => ("mapped", "encoding_mac_roman"),
            GlyphRoute::Missing => ("missing", "missing"),
            GlyphRoute::UnsupportedNameFallback => ("unsupported", "name_fallback"),
        };
        let gid = choice
            .glyph_id()
            .map_or_else(|| "null".into(), |g| g.get().to_string());
        let query = choice
            .lookup_code()
            .map_or_else(|| "null".into(), |q| q.to_string());
        append(&mut out,format!("{{\"code\":{},\"status\":\"{status}\",\"gid\":{gid},\"route\":\"{route}\",\"lookup_code\":{query},\"outline\":",choice.source()).as_bytes())?;
        if choice.glyph_id().is_some() {
            let glyph = session.decode(choice.source()).map_err(|e| e.to_string())?;
            super::cli_outline::outline_json(&mut out, &glyph)?;
        } else {
            append(&mut out, b"null")?;
        }
        append(&mut out, b"}")?;
    }
    let l = loader.usage();
    let c = outlines.map().usage();
    let o = session.usage();
    append(&mut out,format!("],\"usage\":{{\"load_attempts\":{},\"reference_hops\":{},\"metadata_items\":{},\"encoded_bytes\":{},\"decoded_work_bytes\":{},\"name_bytes\":{},\"cmap_queries\":{},\"cmap_work\":{},\"code_attempts\":{},\"outline_attempts\":{},\"outline_work\":{}}}}}\n",l.attempts,l.reference_hops,l.metadata_items,l.encoded_bytes,l.decoded_work_bytes,l.name_bytes,c.queries,c.work,session.code_attempts(),o.decode_attempts,o.work).as_bytes())?;
    Ok(out)
}
