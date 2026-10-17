/*!
A crate for rendering PDF files.

This crate allows you to render pages of a PDF file into bitmaps. It is supposed to be relatively
lightweight, since we do not have any dependencies on the GPU. All the rendering happens on the CPU.

The ultimate goal of this crate is to be a *feature-complete* and *performant* PDF rasterizer.
With that said, we are currently still very far away from reaching that goal: So far, no effort
has been put into performance optimizations, as we are still working on implementing missing features.
However, this crate is currently the most comprehensive and feature-complete
implementation of a PDF rasterizer in pure Rust. This claim is supported by the fact that we currently
include over 1000 PDF files in our regression test suite. The majority of those have been scraped
from the `pdf.js` and `PDFBOX` test suites and therefore represent a very large and diverse sample
of PDF files.

As mentioned, there are still some serious limitations, including lack of support for
encrypted/password-protected PDF files, blending and isolation, knockout groups as well as a range
of smaller features such as color key masking. But you should be able to render the vast majority
of PDF files without too many issues.

## Safety
This crate forbids unsafe code via a crate-level attribute.

## Examples
For usage examples, see the [example](https://github.com/LaurenzV/hayro/tree/master/hayro/examples) in
the GitHub repository.

## Cargo features
This crate has one optional feature:
- `embed-fonts`: See the description of [`hayro-interpret`](https://docs.rs/hayro-interpret/latest/hayro_interpret/#cargo-features) for more information.
*/

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use fearless_simd::Level;
use hayro_interpret::Device;
use hayro_interpret::FillRule;
use hayro_interpret::InterpreterCache;
use hayro_interpret::InterpreterSettings;
use hayro_interpret::font::GlyphRun;
use hayro_interpret::hayro_syntax::Pdf;
use hayro_interpret::hayro_syntax::page::Page;
use hayro_interpret::util::{RectExt, TransformExt};
use hayro_interpret::{BlendMode, Context, DrawMode, DrawProps, ImageDrawProps, SoftMask};
use hayro_interpret::{ClipPath, interpret_page};
use kurbo::{Affine, BezPath, Rect, Shape};
use pic_scale::Scaler;
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::sync::Arc;

pub use hayro_interpret;
// Tandem patch: the path budget.
pub use path::MAX_PATH_BYTES;
pub use hayro_interpret::hayro_syntax;
pub use kurbo;
pub use vello_cpu;

use vello_cpu::color::palette::css::{TRANSPARENT, WHITE};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::peniko::{Compose, Fill, Mix};
use vello_cpu::{Mask, Pixmap, RenderContext, peniko};

mod clip;
mod glyph;
mod image;
mod mask;
mod paint;
mod path;

pub(crate) struct GlobalState {
    level: Level,
    outline_cache: Rc<RefCell<FxHashMap<u128, Rc<BezPath>>>>,
    scaler: Scaler,
    force_image_interpolation: bool,
    /// Tandem patch: pixels of the pixmaps made so far and the most they
    /// may hold, the scene's estimated bytes and the most it may hold,
    /// whether something did not fit (`RenderCache::over_budget`), and the
    /// pixmaps repeated draws share.
    pixmap_pixels: std::cell::Cell<f64>,
    pixmap_budget: f64,
    scene_bytes: std::cell::Cell<f64>,
    scene_budget: f64,
    over_budget: Rc<std::cell::Cell<bool>>,
    pub(crate) shared: RefCell<FxHashMap<SharedKey, SharedPixmap>>,
}

// Tandem patch: vello keeps every image, pattern, shading and soft-mask
// pixmap a page paints until the page is composed, so their sum is bounded
// per render.
/// The pixels a render's pixmaps hold together by default (512 MiB of RGBA;
/// a soft mask's byte a pixel counts a quarter).
pub const MAX_RENDER_PIXMAP_PIXELS: f64 = (1u32 << 27) as f64;
/// The fewest pixels a pixmap is reduced to before the budget counts as
/// spent.
const MIN_PIXMAP_PIXELS: f64 = 4096.0;

// Tandem patch: vello also keeps every fill, stroke and clip of a page, as
// strips and commands for each 4-pixel tile row and 256-pixel wide tile it
// covers, until the page is composed: 20,000 page-sized fills of a 600 ×
// 600 page held 540 MB. Their estimated bytes are bounded per render.
/// The estimated bytes a render's scenes hold together by default.
pub const MAX_RENDER_SCENE_BYTES: f64 = (1u64 << 30) as f64;
/// Estimated scene bytes of one drawing operation, of each tile row it
/// covers, and of each wide tile in such a row (measured on vello_cpu 0.3).
const SCENE_OP_BYTES: f64 = 96.0;
const SCENE_ROW_BYTES: f64 = 64.0;
const SCENE_TILE_BYTES: f64 = 32.0;

/// Tandem patch: what a shared pixmap depends on: the image (or shading)
/// key, the pixmap's size and how it was made.
pub(crate) type SharedKey = (u128, u32, u32, u8);
/// A shared pixmap, its sampling quality and its transform within the
/// space it was drawn in.
pub(crate) type SharedPixmap = (Arc<Pixmap>, peniko::ImageQuality, Affine);

impl GlobalState {
    pub(crate) fn new(cache: &RenderCache<'_>, settings: &RenderSettings) -> Self {
        Self {
            level: Level::new(),
            outline_cache: cache.outline_cache.clone(),
            scaler: Scaler::new(image::RESAMPLING_FUNCTION),
            force_image_interpolation: settings.force_image_interpolation,
            pixmap_pixels: std::cell::Cell::new(0.0),
            pixmap_budget: settings.max_pixmap_pixels,
            scene_bytes: std::cell::Cell::new(0.0),
            scene_budget: settings.max_scene_bytes,
            over_budget: cache.over_budget.clone(),
            shared: RefCell::new(FxHashMap::default()),
        }
    }

    /// Tandem patch: the scale, at most 1, that keeps a `width` × `height`
    /// pixmap within what remains of the pixmap budget, whose pixels are
    /// then counted; `None` (the render is over budget) when less than
    /// MIN_PIXMAP_PIXELS remain for it.
    pub(crate) fn pixmap_fit(&self, width: f64, height: f64) -> Option<f64> {
        let used = self.pixmap_pixels.get();
        let wanted = (width * height).max(1.0);
        let left = self.pixmap_budget - used;
        if wanted <= left {
            self.pixmap_pixels.set(used + wanted);
            Some(1.0)
        } else if left >= MIN_PIXMAP_PIXELS && wanted.is_finite() {
            self.pixmap_pixels.set(self.pixmap_budget);
            Some((left / wanted).sqrt())
        } else {
            self.over_budget.set(true);
            None
        }
    }

    /// Tandem patch: whether `pixels` that cannot be reduced fit what
    /// remains of the pixmap budget (counted when they do).
    pub(crate) fn pixmap_charge(&self, pixels: f64) -> bool {
        let used = self.pixmap_pixels.get();
        if pixels <= self.pixmap_budget - used {
            self.pixmap_pixels.set(used + pixels);
            true
        } else {
            self.over_budget.set(true);
            false
        }
    }
}

pub(crate) struct Renderer<'a> {
    global: &'a GlobalState,
    pub(crate) ctx: &'a mut RenderContext,
    pub(crate) inside_pattern: bool,
    pub(crate) soft_mask_cache: FxHashMap<u128, Option<Mask>>,
    pub(crate) in_type3_glyph: bool,
    /// Tandem patch: the shared key of the image `draw_image` draws next.
    pub(crate) image_key: Option<u128>,
}

impl<'r> Renderer<'r> {
    /// Tandem patch: whether drawing over `bbox` (in the current transform's
    /// space, grown by `margin` pixels) fits what remains of the render's
    /// scene budget; counted when it does, else the render is over budget.
    pub(crate) fn scene_fits(&self, bbox: Rect, margin: f64) -> bool {
        let (width, height) = (f64::from(self.ctx.width()), f64::from(self.ctx.height()));
        let area = self
            .ctx
            .transform()
            .transform_rect_bbox(bbox)
            .inflate(margin, margin)
            .intersect(Rect::new(0.0, 0.0, width, height));
        let rows = (area.height().max(0.0) / 4.0).ceil() + 1.0;
        let tiles = (area.width().max(0.0) / 256.0).ceil() + 1.0;
        let bytes = SCENE_OP_BYTES + rows * (SCENE_ROW_BYTES + SCENE_TILE_BYTES * tiles);
        let used = self.global.scene_bytes.get();
        // Non-finite geometry counts as the whole render.
        let bytes = if bytes.is_finite() { bytes } else { self.global.scene_budget };
        if used + bytes <= self.global.scene_budget {
            self.global.scene_bytes.set(used + bytes);
            true
        } else {
            self.global.over_budget.set(true);
            false
        }
    }

    pub(crate) fn new(ctx: &'r mut RenderContext, global: &'r GlobalState) -> Self {
        Self {
            global,
            ctx,
            inside_pattern: false,
            soft_mask_cache: FxHashMap::default(),
            in_type3_glyph: false,
            image_key: None,
        }
    }

    fn child_context(&self, width: u16, height: u16) -> RenderContext {
        RenderContext::new_with(width, height, derive_settings(self.ctx.render_settings()))
    }

    fn apply_draw_props(&mut self, props: &DrawProps<'_>) {
        self.ctx.set_transform(props.transform);
        self.apply_soft_mask(props.soft_mask.as_ref());
        self.ctx
            .set_blend_mode(convert_blend_mode(props.blend_mode));
    }

    fn apply_image_props(&mut self, props: &ImageDrawProps<'_>) {
        self.ctx.set_transform(props.transform);
        self.apply_soft_mask(props.soft_mask.as_ref());
        self.ctx
            .set_blend_mode(convert_blend_mode(props.blend_mode));
    }
}

impl<'a, 'r> Device<'a> for Renderer<'r> {
    fn draw_image(&mut self, image: hayro_interpret::Image<'a, '_>, props: ImageDrawProps<'a>) {
        Self::draw_pdf_image(self, image, props);
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        Self::push_clip_path(self, clip_path);
    }

    fn push_clip_rect(&mut self, rect: &Rect) {
        Self::push_clip_rect(self, rect);
    }

    fn push_transparency_group(
        &mut self,
        opacity: f32,
        mask: Option<SoftMask<'a>>,
        blend_mode: BlendMode,
    ) {
        Self::push_transparency_group(self, opacity, mask, blend_mode);
    }

    fn pop_clip(&mut self) {
        Self::pop_clip(self);
    }

    fn pop_transparency_group(&mut self) {
        Self::pop_transparency_group(self);
    }

    fn draw_path(&mut self, path: &BezPath, props: DrawProps<'a>, draw_mode: &DrawMode) {
        Self::draw_path(self, path, props, draw_mode);
    }

    fn draw_rect(&mut self, rect: &Rect, props: DrawProps<'a>, draw_mode: &DrawMode) {
        Self::draw_rect(self, rect, props, draw_mode);
    }

    fn draw_glyph_run(
        &mut self,
        glyph_run: &GlyphRun<'_, 'a>,
        props: DrawProps<'a>,
        draw_mode: &DrawMode,
    ) {
        Self::draw_glyph_run(self, glyph_run, props, draw_mode);
    }
}

/// A cache used by the renderer.
///
/// Ideally, such a cache should be constructed once per PDF and then reused across
/// multiple render invocations on the same document.
#[derive(Clone, Default)]
pub struct RenderCache<'a> {
    pub(crate) interpreter_cache: InterpreterCache<'a>,
    pub(crate) outline_cache: Rc<RefCell<FxHashMap<u128, Rc<BezPath>>>>,
    /// Tandem patch: whether the last render passed its pixmap budget.
    pub(crate) over_budget: Rc<std::cell::Cell<bool>>,
}

impl<'a> RenderCache<'a> {
    /// Create a new render cache.
    pub fn new() -> Self {
        Self {
            interpreter_cache: InterpreterCache::new(),
            outline_cache: Rc::new(RefCell::new(FxHashMap::default())),
            over_budget: Rc::default(),
        }
    }

    /// Tandem patch: whether the last render with this cache passed
    /// [`RenderSettings::max_pixmap_pixels`] or
    /// [`RenderSettings::max_scene_bytes`], or the interpreter's budgets for
    /// sampled function tables and content: something it would have drawn
    /// was left out, so the page is incomplete.
    pub fn over_budget(&self) -> bool {
        self.over_budget.get() || self.interpreter_cache.over_budget()
    }

    /// Tandem patch: why the content of the last render with this cache was
    /// refused (decoded content held or read past the interpreter's
    /// budget), if it was; the render is over budget then.
    pub fn content_refusal(&self) -> Option<hayro_interpret::ContentRefusal> {
        self.interpreter_cache.content_refusal()
    }

    /// Tandem patch: the most decoded content bytes the last render with
    /// this cache held at once.
    pub fn content_peak(&self) -> usize {
        self.interpreter_cache.content_peak()
    }
}

/// Settings for rendering a page.
#[derive(Clone, Copy)]
pub struct RenderSettings {
    /// Whether all images should forcibly be rendered with bilinear interpolation.
    pub force_image_interpolation: bool,
    /// Tandem patch: the pixels the image, pattern, shading and soft-mask
    /// pixmaps of one render hold together; a pixmap past it is drawn
    /// reduced to fit, or left out once too little remains
    /// ([`RenderCache::over_budget`]).
    pub max_pixmap_pixels: f64,
    /// Tandem patch: the estimated bytes vello keeps for the fills, strokes
    /// and clips of one render; drawing past it is left out
    /// ([`RenderCache::over_budget`]).
    pub max_scene_bytes: f64,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            force_image_interpolation: false,
            max_pixmap_pixels: MAX_RENDER_PIXMAP_PIXELS,
            max_scene_bytes: MAX_RENDER_SCENE_BYTES,
        }
    }
}

/// Settings for the output pixmap.
#[derive(Clone, Copy)]
pub struct PixmapSettings {
    /// Horizontal scale factor.
    pub x_scale: f32,
    /// Vertical scale factor.
    pub y_scale: f32,
    /// Background color.
    pub bg_color: AlphaColor<Srgb>,
}

impl Default for PixmapSettings {
    fn default() -> Self {
        Self {
            x_scale: 1.0,
            y_scale: 1.0,
            bg_color: TRANSPARENT,
        }
    }
}

/// Render a page to a pixmap with the given settings.
///
/// This is a "simplified" method in case all you want to achieve is rendering a
/// page with a certain scale factor.
///
/// If you want to have more control over how exactly the page is rendered (e.g. with
/// an arbitrary transform or into a custom pixel buffer), use [`render_into`]
pub fn render<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    interpreter_settings: &InterpreterSettings,
    render_settings: &RenderSettings,
    pixmap_settings: &PixmapSettings,
) -> Pixmap {
    let (width, height) = page.render_dimensions();
    let mut ctx = RenderContext::new(
        (width * pixmap_settings.x_scale) as u16,
        (height * pixmap_settings.y_scale) as u16,
    );
    let transform = Affine::scale_non_uniform(
        pixmap_settings.x_scale as f64,
        pixmap_settings.y_scale as f64,
    ) * page.initial_transform(true).to_kurbo();
    render_into(
        page,
        cache,
        interpreter_settings,
        render_settings,
        &mut ctx,
        transform,
    );
    ctx.flush();

    let mut pixmap = Pixmap::new(ctx.width(), ctx.height());
    ctx.render_with(
        &mut pixmap,
        &mut vello_cpu::Resources::default(),
        vello_cpu::RasterizerSettings {
            target_init: vello_cpu::TargetInit::Clear(pixmap_settings.bg_color),
            ..Default::default()
        },
    );
    pixmap
}

/// Render a page into the given [`RenderContext`] with the
/// given transform.
///
/// See the [following example](https://github.com/LaurenzV/hayro/blob/main/hayro/examples/render.rs)
/// if you are unsure how to call this method.
pub fn render_into<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    interpreter_settings: &InterpreterSettings,
    render_settings: &RenderSettings,
    ctx: &mut RenderContext,
    transform: Affine,
) {
    let mut state = Context::new(
        transform,
        Rect::new(0.0, 0.0, ctx.width() as f64, ctx.height() as f64),
        &cache.interpreter_cache,
        page.xref(),
        interpreter_settings.clone(),
    );
    ctx.take_current_state();
    ctx.reset_mask();
    ctx.reset_filter_effect();

    cache.over_budget.set(false);
    let global = GlobalState::new(cache, render_settings);
    let mut device = Renderer::new(ctx, &global);
    let mut clip_path = page.intersected_crop_box().to_kurbo().to_path(0.1);
    clip_path.apply_affine(transform);
    device.push_clip_path(&ClipPath {
        path: clip_path,
        fill: FillRule::NonZero,
    });

    interpret_page(page, &mut state, &mut device);

    device.pop_clip();
}

// Just a convenience method for testing.
#[doc(hidden)]
pub fn render_pdf(
    pdf: &Pdf,
    scale: f32,
    settings: InterpreterSettings,
    range: Option<RangeInclusive<usize>>,
) -> Option<Vec<Pixmap>> {
    let cache = RenderCache::new();
    let rendered = pdf
        .pages()
        .iter()
        .enumerate()
        .flat_map(|(idx, page)| {
            if range.clone().is_some_and(|range| !range.contains(&idx)) {
                return None;
            }

            let pixmap = render(
                page,
                &cache,
                &settings,
                &RenderSettings::default(),
                &PixmapSettings {
                    x_scale: scale,
                    y_scale: scale,
                    bg_color: WHITE,
                },
            );

            Some(pixmap)
        })
        .collect();

    Some(rendered)
}

pub(crate) fn derive_settings(settings: &vello_cpu::RenderSettings) -> vello_cpu::RenderSettings {
    vello_cpu::RenderSettings {
        num_threads: 0,
        ..*settings
    }
}

fn convert_fill_rule(fill_rule: FillRule) -> Fill {
    match fill_rule {
        FillRule::NonZero => Fill::NonZero,
        FillRule::EvenOdd => Fill::EvenOdd,
    }
}

fn convert_blend_mode(blend_mode: BlendMode) -> peniko::BlendMode {
    let mix = match blend_mode {
        BlendMode::Normal => Mix::Normal,
        BlendMode::Multiply => Mix::Multiply,
        BlendMode::Screen => Mix::Screen,
        BlendMode::Overlay => Mix::Overlay,
        BlendMode::Darken => Mix::Darken,
        BlendMode::Lighten => Mix::Lighten,
        BlendMode::ColorDodge => Mix::ColorDodge,
        BlendMode::ColorBurn => Mix::ColorBurn,
        BlendMode::HardLight => Mix::HardLight,
        BlendMode::SoftLight => Mix::SoftLight,
        BlendMode::Difference => Mix::Difference,
        BlendMode::Exclusion => Mix::Exclusion,
        BlendMode::Hue => Mix::Hue,
        BlendMode::Saturation => Mix::Saturation,
        BlendMode::Color => Mix::Color,
        BlendMode::Luminosity => Mix::Luminosity,
    };

    peniko::BlendMode::new(mix, Compose::SrcOver)
}
