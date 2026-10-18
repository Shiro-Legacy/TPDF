// SPDX-License-Identifier: MIT
//! Page rasterization with hayro into a white RGBA8 image.
use super::walk::{self, Scope};
use super::{PageFrame, ViewDocument, ViewError};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{PixmapMut, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings, render_into};
use hayro_interpret::InterpreterWarning;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The largest image `render` produces, in pixels (256 MiB of RGBA).
pub const MAX_RENDER_PIXELS: u64 = 1 << 26;

/// The refusal of a page past hayro's budgets (`RenderBudgets`).
const OVER_BUDGET: ViewError =
    ViewError::TooComplex("page drawing is larger than the render's limit");

impl ViewError {
    /// Whether a render was refused for what its scale costs: its drawing
    /// passed the render's budgets (`RenderBudgets`: fills, strokes, clips
    /// and pixmaps grow with the pixels drawn), or its image could not be
    /// allocated. The page may draw at a lower scale; the walk's refusals
    /// (`TooComplex` for the content itself) do not depend on the scale.
    pub fn scale_bound(&self) -> bool {
        matches!(self, Self::BadScale) || *self == OVER_BUDGET
    }
}

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

/// What one render may hold: the pixels of its image, pattern, shading and
/// soft-mask pixmaps together (past it a pixmap is drawn reduced to fit,
/// and the render fails once too little remains), the estimated bytes of
/// its fills, strokes and clips (past it the render fails), and the samples
/// one image decodes to (past it an image drawn smaller is decoded reduced,
/// else the render fails). `Default` is the app's.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub struct RenderBudgets {
    pub pixmap_pixels: f64,
    pub scene_bytes: f64,
    pub image_samples: u64,
}

impl Default for RenderBudgets {
    fn default() -> Self {
        Self {
            pixmap_pixels: hayro::MAX_RENDER_PIXMAP_PIXELS,
            scene_bytes: hayro::MAX_RENDER_SCENE_BYTES,
            image_samples: hayro_interpret::MAX_IMAGE_SAMPLES,
        }
    }
}

/// What one render held. For tests of the budgets.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStats {
    /// The most decoded content bytes hayro held at once (its content
    /// budget, `hayro_interpret::MAX_HELD_CONTENT_BYTES`); 0 when the walk
    /// refused the page before it was interpreted.
    pub content_peak: usize,
    /// The most bytes one image decoded reduced (too large to decode whole)
    /// kept for its own buffers: a row, the block sums, the reduced image.
    pub image_working_peak: usize,
    /// The bytes of fills, strokes and clips the render estimated vello
    /// kept (`RenderBudgets::scene_bytes` bounds it).
    pub scene_bytes: f64,
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
        let budgets = RenderBudgets::default();
        self.render_with_budgets(page, scale, opts, budgets)
    }

    /// [`Self::render`] within `budgets` instead of the app's. For tests of
    /// the budgets.
    #[doc(hidden)]
    pub fn render_with_budgets(
        &self,
        page: usize,
        scale: f64,
        opts: RenderOptions,
        budgets: RenderBudgets,
    ) -> Result<RgbaImage, ViewError> {
        self.render_with_stats(page, scale, opts, budgets).0
    }

    /// [`Self::render_with_budgets`], and what the render held. For tests of
    /// the budgets.
    #[doc(hidden)]
    pub fn render_with_stats(
        &self,
        page: usize,
        scale: f64,
        opts: RenderOptions,
        budgets: RenderBudgets,
    ) -> (Result<RgbaImage, ViewError>, RenderStats) {
        let mut stats = RenderStats::default();
        let result = self.render_measured(page, scale, opts, budgets, &mut stats);
        (result, stats)
    }

    fn render_measured(
        &self,
        page: usize,
        scale: f64,
        opts: RenderOptions,
        budgets: RenderBudgets,
        stats: &mut RenderStats,
    ) -> Result<RgbaImage, ViewError> {
        let page = self.page(page)?;
        let frame = PageFrame::new(page);
        let (width, height) = pixel_size(&frame, scale)?;
        walk::check_page(
            page,
            &self.font_sizes,
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
        let mut settings = super::fonts::settings(opts.annotations, true);
        settings.max_image_samples = budgets.image_samples;
        // An image past its decoding budget, or pixmaps past the render's,
        // leave the page incomplete: a render error, not a partial page.
        let over_budget = Arc::new(AtomicBool::new(false));
        let flag = over_budget.clone();
        settings.warning_sink = Arc::new(move |warning| {
            if matches!(warning, InterpreterWarning::BudgetExceeded) {
                flag.store(true, Ordering::Relaxed);
            }
        });
        // ponytail: one RenderCache per call; hayro's cache borrows the Pdf,
        // so keeping it would make ViewDocument self-referential.
        let cache = RenderCache::new();
        let render_settings = RenderSettings {
            max_pixmap_pixels: budgets.pixmap_pixels,
            max_scene_bytes: budgets.scene_bytes,
            ..RenderSettings::default()
        };
        render_into(
            page,
            &cache,
            &settings,
            &render_settings,
            &mut ctx,
            Affine::scale(scale) * frame.ctm,
        );
        stats.content_peak = cache.content_peak();
        stats.image_working_peak = cache.reduced_working_peak();
        stats.scene_bytes = cache.scene_bytes();
        // Content hayro refused to decode or read past its budget (pattern
        // cells and glyphs inherited into forms, which the walk does not
        // follow) was left out.
        if let Some(why) = cache.content_refusal() {
            return Err(walk::refused(why));
        }
        if cache.over_budget() || over_budget.load(Ordering::Relaxed) {
            return Err(OVER_BUDGET);
        }
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

/// The longest image edge, the last whole 256-pixel wide tile: vello rounds
/// its tiles up to multiples of four pixels in 16-bit coordinates and
/// panicked drawing at the end of a 65,535-pixel edge.
pub const MAX_RENDER_EDGE: u16 = 65280;

/// A product this little above a whole number of pixels rounds down to it
/// (spec 04, 5.2): 612 × 1.5 is 918 pixels even when floating point gives
/// 918.0000000001.
const PIXEL_EPSILON: f64 = 1e-3;

/// The image size of `frame` at `scale` (spec 04, 5.2): each side the
/// smallest whole number of pixels that holds it, within `PIXEL_EPSILON`.
fn pixel_size(frame: &PageFrame, scale: f64) -> Result<(u16, u16), ViewError> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(ViewError::BadScale);
    }
    let side = |points: f64| -> Result<u16, ViewError> {
        let product = points * scale;
        if !product.is_finite() || product > f64::from(MAX_RENDER_EDGE) + 1.0 {
            return Err(ViewError::BadScale);
        }
        let floor = product.floor();
        let pixels = if product - floor <= PIXEL_EPSILON {
            floor
        } else {
            product.ceil()
        };
        match pixels {
            p if p >= 1.0 && p <= f64::from(MAX_RENDER_EDGE) => Ok(p as u16),
            _ => Err(ViewError::BadScale),
        }
    };
    let (width, height) = (
        side(f64::from(frame.width))?,
        side(f64::from(frame.height))?,
    );
    if u64::from(width) * u64::from(height) > MAX_RENDER_PIXELS {
        return Err(ViewError::BadScale);
    }
    Ok((width, height))
}
