// SPDX-License-Identifier: AGPL-3.0-or-later
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::page_geometry::PageGeometry;
use tandem_pdf_engine::pages::{PageLimits, collect_pages};
pub(super) fn inspect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let document = Document::open(bytes, DocumentLimits::default()).map_err(|e| e.to_string())?;
    let pages = collect_pages(&document, PageLimits::default()).map_err(|e| e.to_string())?;
    let mut output = Vec::new();
    super::append_inspection_output(&mut output, b"{\"pages\":[")?;
    for (index, page) in pages.iter().enumerate() {
        let geometry = PageGeometry::from_page_info(page).map_err(|e| e.to_string())?;
        if index > 0 {
            super::append_inspection_output(&mut output, b",")?;
        }
        let matrix = geometry.user_to_page().coefficients();
        let bounds = geometry.bounds().bounds();
        super::append_inspection_output(
            &mut output,
            format!(
                "{{\"matrix\":[{},{},{},{},{},{}],\"bounds\":[{},{},{},{}]}}",
                matrix[0],
                matrix[1],
                matrix[2],
                matrix[3],
                matrix[4],
                matrix[5],
                bounds[0],
                bounds[1],
                bounds[2],
                bounds[3]
            )
            .as_bytes(),
        )?;
    }
    super::append_inspection_output(&mut output, b"]}\n")?;
    Ok(output)
}
