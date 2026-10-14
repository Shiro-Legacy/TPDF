// SPDX-License-Identifier: AGPL-3.0-or-later
//! Raw PPM inspection for the deliberately narrow optional raster subset.
use tandem_pdf_engine::content::ContentLimits;
use tandem_pdf_engine::geometry::{Affine, DeviceSpace, PageSpace};
use tandem_pdf_engine::path::{PathInitialState, PathLimits, interpret_path_content};
use tandem_pdf_engine::raster::{RasterLimits, RasterTarget, render_orthogonal_raster};
pub(super) fn inspect(bytes: &[u8], dimensions: &str) -> Result<Vec<u8>, String> {
    let (width, height) = dimensions
        .split_once('x')
        .ok_or("raster needs WIDTHxHEIGHT")?;
    let width = width.parse::<u32>().map_err(|_| "invalid raster width")?;
    let height = height.parse::<u32>().map_err(|_| "invalid raster height")?;
    let program = interpret_path_content(
        bytes,
        PathInitialState::default(),
        ContentLimits::default(),
        PathLimits::default(),
    )
    .map_err(|e| e.to_string())?;
    let page_to_device = Affine::<PageSpace, DeviceSpace>::from_pdf([1., 0., 0., 1., 0., 0.])
        .map_err(|e| e.to_string())?;
    let image = render_orthogonal_raster(
        &program,
        RasterTarget {
            width,
            height,
            page_to_device,
        },
        RasterLimits::default(),
    )
    .map_err(|e| e.to_string())?;
    let mut output = Vec::new();
    super::append_inspection_output(
        &mut output,
        format!("P6\n{} {}\n255\n", image.width(), image.height()).as_bytes(),
    )?;
    for pixel in image.pixels().chunks_exact(4) {
        if pixel[3] != 255 {
            return Err("opaque raster unexpectedly contains transparency".into());
        }
        super::append_inspection_output(&mut output, &pixel[..3])?;
    }
    Ok(output)
}
