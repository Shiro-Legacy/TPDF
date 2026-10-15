// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page rasterization with hayro into a white RGBA8 image.
use super::walk::{self, Scope};
use super::{PageFrame, ViewDocument, ViewError};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{PixmapMut, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings, render_into};

/// The largest image `render` produces, in pixels (256 MiB of RGBA).
pub const MAX_RENDER_PIXELS: u64 = 1 << 26;

#[derive(Clone, Copy, Debug)]
pub struct RenderOptions {
    /// Draw annotation and widget appearance streams.
    pub annotations: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self { annotations: true }
    }
}

/// Straight RGBA8 pixels, rows top to bottom, alpha always 255.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl ViewDocument {
    /// Render `page` at `scale` pixels per point.
    pub fn render(
        &self,
        page: usize,
        scale: f64,
        opts: RenderOptions,
    ) -> Result<RgbaImage, ViewError> {
        let page = self.page(page)?;
        let frame = PageFrame::new(page);
        let (width, height) = pixel_size(&frame, scale)?;
        walk::check_page(
            page,
            Scope {
                paint: true,
                annotations: opts.annotations,
            },
        )?;
        let len = usize::from(width) * usize::from(height) * 4;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(len)
            .map_err(|_| ViewError::BadScale)?;
        pixels.resize(len, 0);
        let mut ctx = RenderContext::new(width, height);
        let settings = super::fonts::settings(opts.annotations, true);
        // ponytail: one RenderCache per call; hayro's cache borrows the Pdf,
        // so keeping it would make ViewDocument self-referential.
        let cache = RenderCache::new();
        render_into(
            page,
            &cache,
            &settings,
            &RenderSettings::default(),
            &mut ctx,
            Affine::scale(scale) * frame.ctm,
        );
        ctx.flush();
        let target = PixmapMut::new(width, height, &mut pixels).ok_or(ViewError::BadScale)?;
        ctx.render_with(
            target,
            &mut Resources::default(),
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
                ..RasterizerSettings::default()
            },
        );
        Ok(RgbaImage {
            width: u32::from(width),
            height: u32::from(height),
            pixels,
        })
    }
}

/// MuPDF's pixmap bounds: floor/ceil of the scaled page with a 0.001 tolerance,
/// in f32, within [`MAX_RENDER_PIXELS`].
fn pixel_size(frame: &PageFrame, scale: f64) -> Result<(u16, u16), ViewError> {
    let s = scale as f32;
    if !(s.is_finite() && s > 0.0) {
        return Err(ViewError::BadScale);
    }
    let edge = |v: f32| {
        let px = (v * s - 0.001).ceil();
        (px >= 1.0 && px <= f32::from(u16::MAX)).then_some(px as u16)
    };
    edge(frame.width)
        .zip(edge(frame.height))
        .filter(|&(w, h)| u64::from(w) * u64::from(h) <= MAX_RENDER_PIXELS)
        .ok_or(ViewError::BadScale)
}
