// SPDX-License-Identifier: AGPL-3.0-or-later
//! Atomic PPM output for the bounded page raster subset.
use tandem_pdf_engine::{
    document::{Document, DocumentLimits},
    page_glyph::{PageGlyphLimits, PageGlyphSession, PageGlyphSessionLimits, PageOutlinePolicy},
    page_program::PageMetricPolicy,
    page_raster::{PageRasterLimits, PageRasterSession, PageRasterSessionLimits, RasterTarget},
};
pub(super) fn inspect(input: &[u8], selector: &str) -> Result<Vec<u8>, String> {
    let (page, dpi) = selector
        .split_once('@')
        .ok_or("page-render requires N@DPI")?;
    let index = page
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .ok_or("page-render requires a positive one-based page number")?;
    let dpi = dpi.parse::<f64>().map_err(|_| "invalid render DPI")?;
    if !dpi.is_finite() || dpi <= 0. {
        return Err("render DPI must be finite and positive".into());
    }
    let doc = Document::open(input, DocumentLimits::default()).map_err(|e| e.to_string())?;
    let mut limits = PageGlyphSessionLimits::default();
    limits.page.metric_policy = PageMetricPolicy::AllowPinnedBase14;
    limits.outline_policy = PageOutlinePolicy::AllowPinnedBase14;
    let mut glyphs = PageGlyphSession::new(&doc, limits).map_err(|e| e.to_string())?;
    if index >= glyphs.page_count() {
        return Err("page index outside document".into());
    }
    let page = glyphs
        .trace_page(index, PageGlyphLimits::default())
        .map_err(|e| e.to_string())?;
    let geometry = page.page_program().geometry();
    let page_to_device = geometry.page_to_device(dpi).map_err(|e| e.to_string())?;
    let bounds = page_to_device
        .transform_bounds(geometry.bounds())
        .and_then(|r| r.outward_raster_bounds())
        .map_err(|e| e.to_string())?;
    if bounds.bounds()[0] != 0 || bounds.bounds()[1] != 0 {
        return Err("page raster origin invariant".into());
    }
    let width = u32::try_from(bounds.width()).map_err(|_| "page raster width overflow")?;
    let height = u32::try_from(bounds.height()).map_err(|_| "page raster height overflow")?;
    let mut session = PageRasterSession::new(PageRasterSessionLimits::default());
    let image = session
        .render(
            &page,
            RasterTarget {
                width,
                height,
                page_to_device,
            },
            PageRasterLimits::default(),
        )
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    super::append_inspection_output(
        &mut out,
        format!("P6\n{} {}\n255\n", image.width(), image.height()).as_bytes(),
    )?;
    for pixel in image.pixels().chunks_exact(4) {
        if pixel[3] != 255 {
            return Err("opaque page raster unexpectedly contains transparency".into());
        }
        super::append_inspection_output(&mut out, &pixel[..3])?;
    }
    Ok(out)
}
