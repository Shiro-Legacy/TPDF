# hayro 0.8.0, patched for Tandem PDF

This directory is hayro 0.8.0 as published on crates.io (`.crate` checksum
`4675ff5acd7ca41c0c34ccd2a3a154f5c86d8b5ec92bef0a114bfb194969e2af`, the one
`Cargo.lock` pinned before), with the patch below. It is wired in with
`[patch.crates-io]` in `engine/Cargo.toml` and `engine/bridge/Cargo.toml`.
Licence: Apache-2.0 OR MIT (`LICENSE-APACHE`, `LICENSE-MIT`, unchanged). Why a
patch and not a guard in the engine: the panics and allocations happen while
hayro paints, from sizes it derives from the page (pattern steps scaled by the
CTM, image samples against their drawn size), which the engine cannot see or
rewrite before hayro runs without reimplementing that geometry. Every change
is marked `Tandem patch` in the source. Design notes and measurements:
`engine/design/app-corpus-robust.md`; tests: `engine/tests/view_robust.rs`.

## Changes

- `src/paint.rs`, tiling patterns:
  - A step pixmap's edge was `step × scale` cast to `u16` (65,535 for a
    huge step), so a 90,000-unit step asked for a 65,535 × 65,535 pixmap
    (17 GB) and vello panicked snapping tiles at the 16-bit edge
    (`vello_common` `util.rs:237`; pdf.js `tiling-pattern-large-steps.pdf`).
    The step pixmap now holds at most `MAX_TILE_PIXELS` (2^24) and
    `MAX_TILE_EDGE` (16,384) a side, its scale reduced to fit.
  - When the steps leave space between cells and the area drawn meets one
    cell only, that cell is drawn alone at its own size (a padded image placed
    on the lattice) instead of a step-sized pixmap: a writer that paints an
    image through a one-cell pattern kept a 33 MB step pixmap per pattern
    until the page was composed (pdf.js `tiling-pattern-box.pdf`: past
    1.5 GB).
  - A negative step places cells at multiples of its magnitude, unmirrored
    (MuPDF 1.28.1, the specification); hayro mirrored the cell.
  - Step and cell pixmaps count against the render's pixmap budget (below).
  - Shadings are encoded after the area drawn is known and pass it on
    (`ShadingPattern::encode_within`), so meshes are sampled only there.
- `src/paint.rs`, shading textures: a texture counts against the pixmap
  budget (sampled more coarsely to fit), and paints of one shading defined
  by its dictionary (not a mesh) over one area share it (100 paints of a
  function-based shading sampled 100 textures: 2.4 s, +106 MB).
- `src/image.rs`, images:
  - An image pixmap holds at most `MAX_IMAGE_PIXELS` (2^25, 128 MiB of RGBA)
    and 32,767 pixels a side, and counts against the render's budget; a
    larger one is drawn reduced. hayro cut a width past 65,535 to 16 bits and
    vello refused the pixmap.
  - Draws of one image at one size share one pixmap (`image_key`,
    `GlobalState::shared`, keyed by hayro-interpret's `shared_key`): every
    draw made its own and vello kept all of them (a 64 × 64 image drawn
    20,000 times: 426 MB).
  - A reduction of more than 8× starts with a whole-number box filter
    (`box_reduce`, sizes checked: on wasm32 they wrapped), so the resampler's
    work follows its output, not the samples (pdf.js `issue16263.pdf`: 18 s
    → 0.4 s with the decoder's own reduction in hayro-interpret).
  - A resampler failure leaves the resized samples zero instead of
    panicking (`unwrap` on `pic_scale` results).
- `src/mask.rs`: a soft mask (a byte a pixel, kept until the page is
  composed) counts a quarter pixel a pixel against the pixmap budget; past
  it the mask is left out.
- `src/lib.rs`, the render's budgets: vello keeps every pixmap, fill, stroke
  and clip a page paints until the page is composed. `GlobalState` counts
  pixmap pixels against `RenderSettings::max_pixmap_pixels` (default
  `MAX_RENDER_PIXMAP_PIXELS`, 2^27, 512 MiB of RGBA): a pixmap is reduced to
  fit what remains while at least 4,096 pixels remain, then left out. It
  also estimates the scene's bytes from each operation's device bounding box
  against `RenderSettings::max_scene_bytes` (`MAX_RENDER_SCENE_BYTES`, 2^30;
  `Renderer::scene_fits`, called in `path.rs`, `glyph.rs`, `clip.rs` and
  `image.rs`): 20,000 page-sized fills of a 600 × 600 page held 540 MB.
  Whatever does not fit is left out and `RenderCache::over_budget` reports
  it, so the caller can refuse the incomplete page. `RenderSettings` gains
  the two fields and a `Default` impl in place of the derive.

### Integration acceptance review (October 16, 2026)

- `src/path.rs`, `src/clip.rs`, `src/lib.rs`: one fill, stroke or clip may
  cost vello at most `MAX_PATH_BYTES` (2^29) at once, estimated from its
  segments, the lines its curves flatten into and its device length within
  the pixmap (vello culls the rest while flattening) and, for a stroke, its
  dashes along the whole path (kurbo dashes it all first) and about three
  widths of outline for each segment near the pixmap (`path_fits`); past it
  the render is over budget. A dash pattern shorter than half a
  pixel or 0.01 units strokes solid, as MuPDF 1.28.1 does. A 24 KB page
  stroking 2,000,000 tiny segments took 2.3 GB of WASM memory and 8,000,000
  aborted the engine; a 50-byte dash pattern took 1.4 GB natively
  (`engine/design/app-accept-integration.md`, `engine/tests/view_hostile.rs`).
  `RenderCache::over_budget` also reports hayro-interpret's cache budget.

## Diff against the published crate

```diff
diff -ru a/src/clip.rs b/src/clip.rs
--- a/src/clip.rs
+++ b/src/clip.rs
@@ -1,6 +1,6 @@
 use crate::{Renderer, convert_fill_rule};
 use hayro_interpret::{ClipPath, FillRule};
-use kurbo::{Affine, BezPath, Rect};
+use kurbo::{Affine, BezPath, Rect, Shape};
 
 impl Renderer<'_> {
     pub(super) fn push_clip_path_inner(&mut self, clip_path: &BezPath, fill: FillRule) {
@@ -8,7 +8,13 @@
 
         self.ctx.set_fill_rule(convert_fill_rule(fill));
         self.ctx.set_transform(Affine::IDENTITY);
-        self.ctx.push_clip_path(clip_path);
+        // Tandem patch: within the render's scene budget (past it, an empty
+        // clip keeps the clip stack balanced).
+        if self.scene_fits(clip_path.bounding_box(), 0.0) && self.path_fits(clip_path, false) {
+            self.ctx.push_clip_path(clip_path);
+        } else {
+            self.ctx.push_clip_rect(&Rect::ZERO);
+        }
 
         self.ctx.set_transform(old_transform);
     }
diff -ru a/src/glyph.rs b/src/glyph.rs
--- a/src/glyph.rs
+++ b/src/glyph.rs
@@ -130,7 +130,11 @@
                     };
 
                     self.ctx.set_transform(props.transform * glyph.transform());
-                    self.ctx.fill_path(self.cached_outline(outline).as_ref());
+                    let outline = self.cached_outline(outline);
+                    // Tandem patch: within the render's scene budget.
+                    if self.scene_fits(outline.bounding_box(), 0.0) {
+                        self.ctx.fill_path(outline.as_ref());
+                    }
                 }
                 self.finish_run(clip_path);
             }
@@ -138,7 +142,10 @@
                 let outlines = self.transformed_outlines(glyph_run);
                 let clip_path = self.begin_fill_run(props, Self::outline_bbox(&outlines));
                 for outline in &outlines {
-                    self.ctx.fill_path(outline);
+                    // Tandem patch: within the render's scene budget.
+                    if self.scene_fits(outline.bounding_box(), 0.0) {
+                        self.ctx.fill_path(outline);
+                    }
                 }
                 self.finish_run(clip_path);
             }
@@ -160,7 +167,10 @@
                     };
 
                     let outline = glyph.transform() * self.cached_outline(outline).as_ref().clone();
-                    self.ctx.stroke_path(&outline);
+                    // Tandem patch: within the render's scene budget.
+                    if self.scene_fits(outline.bounding_box(), self.stroke_margin()) {
+                        self.ctx.stroke_path(&outline);
+                    }
                 }
                 self.finish_run(clip_path);
             }
@@ -169,7 +179,10 @@
                 let clip_path =
                     self.begin_stroke_run(props, stroke_props, Self::outline_bbox(&outlines));
                 for outline in &outlines {
-                    self.ctx.stroke_path(outline);
+                    // Tandem patch: within the render's scene budget.
+                    if self.scene_fits(outline.bounding_box(), self.stroke_margin()) {
+                        self.ctx.stroke_path(outline);
+                    }
                 }
                 self.finish_run(clip_path);
             }
diff -ru a/src/image.rs b/src/image.rs
--- a/src/image.rs
+++ b/src/image.rs
@@ -6,6 +6,7 @@
 use pic_scale::{
     ImageSize, ImageStore, ImageStoreMut, PicScaleError, Resampling, ResamplingFunction, Scaler,
 };
+use std::hash::{Hash, Hasher};
 use std::sync::Arc;
 use vello_cpu::peniko::{Compose, Fill, ImageQuality, ImageSampler, Mix};
 use vello_cpu::{Image, ImageSource, Mask, PixelMetadata, Pixmap, peniko};
@@ -19,6 +20,68 @@
 // have this problem.
 pub(super) const RESAMPLING_FUNCTION: ResamplingFunction = ResamplingFunction::Hermite;
 
+// Tandem patch: image pixmap bounds.
+
+/// The most pixels an image pixmap holds (128 MiB of RGBA).
+const MAX_IMAGE_PIXELS: f64 = (1u32 << 25) as f64;
+/// The reduction past which the box filter runs first.
+const BOX_REDUCTION: u32 = 8;
+
+/// `data` (`width` × `height` pixels of N bytes) reduced by whole-number box
+/// factors toward `target_width` × `target_height`, leaving the resampler a
+/// reduction of BOX_REDUCTION / 2 to BOX_REDUCTION along an axis reduced more.
+fn box_reduce<const N: usize>(
+    data: Vec<u8>,
+    width: u32,
+    height: u32,
+    target_width: u32,
+    target_height: u32,
+) -> (Vec<u8>, u32, u32) {
+    let factor = |from: u32, to: u32| {
+        let ratio = from / to.max(1);
+        if ratio > BOX_REDUCTION {
+            (ratio / (BOX_REDUCTION / 2)) as usize
+        } else {
+            1
+        }
+    };
+    let (kx, ky) = (factor(width, target_width), factor(height, target_height));
+    let (w, h) = (width as usize, height as usize);
+    let size = w.checked_mul(h).and_then(|p| p.checked_mul(N));
+    if (kx == 1 && ky == 1) || w == 0 || h == 0 || size.is_none_or(|size| data.len() < size) {
+        return (data, width, height);
+    }
+    let (out_w, out_h) = (w.div_ceil(kx), h.div_ceil(ky));
+    let mut sums = vec![0u64; out_w * N];
+    let mut out = Vec::with_capacity(out_w * out_h * N);
+    for by in 0..out_h {
+        sums.iter_mut().for_each(|s| *s = 0);
+        let rows = by * ky..((by + 1) * ky).min(h);
+        let row_count = rows.len();
+        for row in data[rows.start * w * N..rows.end * w * N].chunks_exact(w * N) {
+            for (bx, block) in row.chunks(kx * N).enumerate() {
+                let sum = &mut sums[bx * N..(bx + 1) * N];
+                if N == 1 {
+                    sum[0] += block.iter().map(|v| u64::from(*v)).sum::<u64>();
+                } else {
+                    for pixel in block.chunks_exact(N) {
+                        for (s, v) in sum.iter_mut().zip(pixel) {
+                            *s += u64::from(*v);
+                        }
+                    }
+                }
+            }
+        }
+        for bx in 0..out_w {
+            let count = ((((bx + 1) * kx).min(w) - bx * kx) * row_count) as u64;
+            for s in &sums[bx * N..(bx + 1) * N] {
+                out.push(((s + count / 2) / count) as u8);
+            }
+        }
+    }
+    (out, out_w as u32, out_h as u32)
+}
+
 #[derive(Clone, Copy)]
 enum ImagePixelFormat {
     Luma,
@@ -175,21 +238,36 @@
             ImageSize,
         ) -> Result<Arc<Resampling<u8, N>>, PicScaleError>,
     ) -> Vec<u8> {
+        // Tandem patch: a large reduction starts with a box filter, so the
+        // resampler's work stays proportional to its output; a failure gives
+        // zeros instead of a panic.
+        let (data, src_width, src_height) =
+            box_reduce::<N>(data, src_width, src_height, new_width, new_height);
         let source_size = ImageSize::new(src_width as usize, src_height as usize);
         let target_size = ImageSize::new(new_width as usize, new_height as usize);
-        let src = ImageStore::<u8, N>::from_slice(&data, src_width as usize, src_height as usize)
-            .unwrap();
         let mut out = vec![0; new_width as usize * new_height as usize * N];
-        let mut dst =
-            ImageStoreMut::<u8, N>::from_slice(&mut out, new_width as usize, new_height as usize)
-                .unwrap();
-        let plan = plan(&self.global.scaler, source_size, target_size).unwrap();
-        plan.resample(&src, &mut dst).unwrap();
+        let resampled =
+            ImageStore::<u8, N>::from_slice(&data, src_width as usize, src_height as usize)
+                .ok()
+                .zip(
+                    ImageStoreMut::<u8, N>::from_slice(
+                        &mut out,
+                        new_width as usize,
+                        new_height as usize,
+                    )
+                    .ok(),
+                )
+                .zip(plan(&self.global.scaler, source_size, target_size).ok())
+                .is_some_and(|((src, mut dst), plan)| plan.resample(&src, &mut dst).is_ok());
+        if !resampled {
+            out.fill(0);
+        }
         out
     }
 
     fn draw_image(&mut self, image_data: impl Into<RenderImageData>, alpha_data: Option<LumaData>) {
         let image_data = image_data.into();
+        let key = self.image_key.take();
         let cur_transform = *self.ctx.transform();
         let mut additional_transform = Affine::IDENTITY;
 
@@ -217,8 +295,8 @@
 
         let has_alpha = alpha_data.is_some();
         let mut may_have_transparency = has_alpha;
-        let needs_resize = x_scale < 1.0 || y_scale < 1.0;
-        let (new_width, new_height) = if needs_resize {
+        let mut needs_resize = x_scale < 1.0 || y_scale < 1.0;
+        let (mut new_width, mut new_height) = if needs_resize {
             let w = (img_width as f32 * x_scale)
                 .ceil()
                 .max(1.0)
@@ -234,6 +312,36 @@
         } else {
             (img_width, img_height)
         };
+        // Tandem patch: draws of one image at one size share its pixmap. An
+        // image pixmap holds at most MAX_IMAGE_PIXELS and u16::MAX / 2 pixels
+        // a side, within the render's pixmap budget; a larger one is drawn
+        // reduced, and none once the budget is spent.
+        let shared = key.map(|key| {
+            let kind = u8::from(needs_resize) | u8::from(self.in_type3_glyph) << 1;
+            (key, new_width, new_height, kind)
+        });
+        let hit = shared.and_then(|shared| self.global.shared.borrow().get(&shared).cloned());
+        if let Some((pixmap, quality, local)) = hit {
+            self.draw_pixmap(pixmap, quality, cur_transform * local);
+            return;
+        }
+        let fit = (MAX_IMAGE_PIXELS / (f64::from(new_width) * f64::from(new_height)))
+            .sqrt()
+            .min(f64::from(u16::MAX / 2) / f64::from(new_width))
+            .min(f64::from(u16::MAX / 2) / f64::from(new_height))
+            .min(1.0);
+        let Some(budget) = self
+            .global
+            .pixmap_fit(f64::from(new_width) * fit, f64::from(new_height) * fit)
+        else {
+            return;
+        };
+        let fit = fit * budget;
+        if fit < 1.0 {
+            new_width = ((f64::from(new_width) * fit) as u32).max(1);
+            new_height = ((f64::from(new_height) * fit) as u32).max(1);
+            needs_resize = true;
+        }
 
         // For luma images without alpha, we can resize as single-channel and
         // expand to RGBA afterwards, which is ~4x faster.
@@ -464,7 +572,7 @@
             rgba_data = padded_image;
         }
 
-        let pixmap = Pixmap::from_parts(
+        let pixmap = Arc::new(Pixmap::from_parts(
             rgba_data,
             img_width as u16,
             img_height as u16,
@@ -472,13 +580,13 @@
                 may_have_transparency,
                 ..Default::default()
             },
-        );
+        ));
+        if let Some(shared) = shared {
+            let entry = (pixmap.clone(), quality, additional_transform);
+            self.global.shared.borrow_mut().insert(shared, entry);
+        }
 
-        self.draw_pixmap(
-            Arc::new(pixmap),
-            quality,
-            cur_transform * additional_transform,
-        );
+        self.draw_pixmap(pixmap, quality, cur_transform * additional_transform);
     }
 
     fn draw_pixmap(&mut self, pixmap: Arc<Pixmap>, quality: ImageQuality, transform: Affine) {
@@ -494,9 +602,13 @@
         };
 
         self.ctx.set_transform(transform);
+        // Tandem patch: within the render's scene budget.
+        let rect = Rect::new(0.0, 0.0, width as f64, height as f64);
+        if !self.scene_fits(rect, 1.0) {
+            return;
+        }
         self.ctx.set_paint(image);
-        self.ctx
-            .fill_rect(&Rect::new(0.0, 0.0, width as f64, height as f64));
+        self.ctx.fill_rect(&rect);
     }
 
     pub(super) fn draw_pdf_image<'a>(
@@ -512,6 +624,15 @@
         let (x, y) = x_y_advances(&transform);
         let target_width = (x.length() * image.width() as f64).ceil() as u32;
         let target_height = (y.length() * image.height() as f64).ceil() as u32;
+        // Tandem patch: what the image decodes to at this target, for draws
+        // that may share its pixmap.
+        let shared_key = |key: Option<u128>, color: [u8; 4]| {
+            key.map(|key| {
+                let mut hasher = rustc_hash::FxHasher::default();
+                (key, target_width, target_height, color).hash(&mut hasher);
+                u128::from(hasher.finish()) | u128::from(key as u64) << 64
+            })
+        };
 
         match image {
             hayro_interpret::Image::Stencil(s) => {
@@ -543,6 +664,7 @@
                                 let old_rule = *self.ctx.fill_rule();
                                 self.ctx.set_fill_rule(Fill::NonZero);
 
+                                self.image_key = shared_key(s.shared_key(), color);
                                 self.draw_image(
                                     RenderImageData::Solid(SolidColorImage {
                                         color: [color[0], color[1], color[2]],
@@ -604,7 +726,10 @@
                                 if let Some(clip_path) = clip_path.as_ref() {
                                     self.push_clip_path_inner(clip_path, FillRule::NonZero);
                                 }
-                                self.ctx.fill_rect(&stencil_rect);
+                                // Tandem patch: within the render's scene budget.
+                                if self.scene_fits(stencil_rect, 1.0) {
+                                    self.ctx.fill_rect(&stencil_rect);
+                                }
                                 if clip_path.is_some() {
                                     self.ctx.pop_clip();
                                 }
@@ -622,6 +747,7 @@
                         let (sx, sy) = image.scale_factors();
                         transform *= Affine::scale_non_uniform(sx as f64, sy as f64);
                         self.ctx.set_transform(transform);
+                        self.image_key = shared_key(r.shared_key(), [0; 4]);
                         self.draw_image(image, alpha);
                     },
                     Some((target_width, target_height)),
diff -ru a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -50,8 +50,11 @@
 use std::cell::RefCell;
 use std::ops::RangeInclusive;
 use std::rc::Rc;
+use std::sync::Arc;
 
 pub use hayro_interpret;
+// Tandem patch: the path budget.
+pub use path::MAX_PATH_BYTES;
 pub use hayro_interpret::hayro_syntax;
 pub use kurbo;
 pub use vello_cpu;
@@ -73,8 +76,47 @@
     outline_cache: Rc<RefCell<FxHashMap<u128, Rc<BezPath>>>>,
     scaler: Scaler,
     force_image_interpolation: bool,
+    /// Tandem patch: pixels of the pixmaps made so far and the most they
+    /// may hold, the scene's estimated bytes and the most it may hold,
+    /// whether something did not fit (`RenderCache::over_budget`), and the
+    /// pixmaps repeated draws share.
+    pixmap_pixels: std::cell::Cell<f64>,
+    pixmap_budget: f64,
+    scene_bytes: std::cell::Cell<f64>,
+    scene_budget: f64,
+    over_budget: Rc<std::cell::Cell<bool>>,
+    pub(crate) shared: RefCell<FxHashMap<SharedKey, SharedPixmap>>,
 }
 
+// Tandem patch: vello keeps every image, pattern, shading and soft-mask
+// pixmap a page paints until the page is composed, so their sum is bounded
+// per render.
+/// The pixels a render's pixmaps hold together by default (512 MiB of RGBA;
+/// a soft mask's byte a pixel counts a quarter).
+pub const MAX_RENDER_PIXMAP_PIXELS: f64 = (1u32 << 27) as f64;
+/// The fewest pixels a pixmap is reduced to before the budget counts as
+/// spent.
+const MIN_PIXMAP_PIXELS: f64 = 4096.0;
+
+// Tandem patch: vello also keeps every fill, stroke and clip of a page, as
+// strips and commands for each 4-pixel tile row and 256-pixel wide tile it
+// covers, until the page is composed: 20,000 page-sized fills of a 600 ×
+// 600 page held 540 MB. Their estimated bytes are bounded per render.
+/// The estimated bytes a render's scenes hold together by default.
+pub const MAX_RENDER_SCENE_BYTES: f64 = (1u64 << 30) as f64;
+/// Estimated scene bytes of one drawing operation, of each tile row it
+/// covers, and of each wide tile in such a row (measured on vello_cpu 0.3).
+const SCENE_OP_BYTES: f64 = 96.0;
+const SCENE_ROW_BYTES: f64 = 64.0;
+const SCENE_TILE_BYTES: f64 = 32.0;
+
+/// Tandem patch: what a shared pixmap depends on: the image (or shading)
+/// key, the pixmap's size and how it was made.
+pub(crate) type SharedKey = (u128, u32, u32, u8);
+/// A shared pixmap, its sampling quality and its transform within the
+/// space it was drawn in.
+pub(crate) type SharedPixmap = (Arc<Pixmap>, peniko::ImageQuality, Affine);
+
 impl GlobalState {
     pub(crate) fn new(cache: &RenderCache<'_>, settings: &RenderSettings) -> Self {
         Self {
@@ -82,19 +124,86 @@
             outline_cache: cache.outline_cache.clone(),
             scaler: Scaler::new(image::RESAMPLING_FUNCTION),
             force_image_interpolation: settings.force_image_interpolation,
+            pixmap_pixels: std::cell::Cell::new(0.0),
+            pixmap_budget: settings.max_pixmap_pixels,
+            scene_bytes: std::cell::Cell::new(0.0),
+            scene_budget: settings.max_scene_bytes,
+            over_budget: cache.over_budget.clone(),
+            shared: RefCell::new(FxHashMap::default()),
         }
     }
+
+    /// Tandem patch: the scale, at most 1, that keeps a `width` × `height`
+    /// pixmap within what remains of the pixmap budget, whose pixels are
+    /// then counted; `None` (the render is over budget) when less than
+    /// MIN_PIXMAP_PIXELS remain for it.
+    pub(crate) fn pixmap_fit(&self, width: f64, height: f64) -> Option<f64> {
+        let used = self.pixmap_pixels.get();
+        let wanted = (width * height).max(1.0);
+        let left = self.pixmap_budget - used;
+        if wanted <= left {
+            self.pixmap_pixels.set(used + wanted);
+            Some(1.0)
+        } else if left >= MIN_PIXMAP_PIXELS && wanted.is_finite() {
+            self.pixmap_pixels.set(self.pixmap_budget);
+            Some((left / wanted).sqrt())
+        } else {
+            self.over_budget.set(true);
+            None
+        }
+    }
+
+    /// Tandem patch: whether `pixels` that cannot be reduced fit what
+    /// remains of the pixmap budget (counted when they do).
+    pub(crate) fn pixmap_charge(&self, pixels: f64) -> bool {
+        let used = self.pixmap_pixels.get();
+        if pixels <= self.pixmap_budget - used {
+            self.pixmap_pixels.set(used + pixels);
+            true
+        } else {
+            self.over_budget.set(true);
+            false
+        }
+    }
 }
 
 pub(crate) struct Renderer<'a> {
     global: &'a GlobalState,
     pub(crate) ctx: &'a mut RenderContext,
     pub(crate) inside_pattern: bool,
-    pub(crate) soft_mask_cache: FxHashMap<u128, Mask>,
+    pub(crate) soft_mask_cache: FxHashMap<u128, Option<Mask>>,
     pub(crate) in_type3_glyph: bool,
+    /// Tandem patch: the shared key of the image `draw_image` draws next.
+    pub(crate) image_key: Option<u128>,
 }
 
 impl<'r> Renderer<'r> {
+    /// Tandem patch: whether drawing over `bbox` (in the current transform's
+    /// space, grown by `margin` pixels) fits what remains of the render's
+    /// scene budget; counted when it does, else the render is over budget.
+    pub(crate) fn scene_fits(&self, bbox: Rect, margin: f64) -> bool {
+        let (width, height) = (f64::from(self.ctx.width()), f64::from(self.ctx.height()));
+        let area = self
+            .ctx
+            .transform()
+            .transform_rect_bbox(bbox)
+            .inflate(margin, margin)
+            .intersect(Rect::new(0.0, 0.0, width, height));
+        let rows = (area.height().max(0.0) / 4.0).ceil() + 1.0;
+        let tiles = (area.width().max(0.0) / 256.0).ceil() + 1.0;
+        let bytes = SCENE_OP_BYTES + rows * (SCENE_ROW_BYTES + SCENE_TILE_BYTES * tiles);
+        let used = self.global.scene_bytes.get();
+        // Non-finite geometry counts as the whole render.
+        let bytes = if bytes.is_finite() { bytes } else { self.global.scene_budget };
+        if used + bytes <= self.global.scene_budget {
+            self.global.scene_bytes.set(used + bytes);
+            true
+        } else {
+            self.global.over_budget.set(true);
+            false
+        }
+    }
+
     pub(crate) fn new(ctx: &'r mut RenderContext, global: &'r GlobalState) -> Self {
         Self {
             global,
@@ -102,6 +211,7 @@
             inside_pattern: false,
             soft_mask_cache: FxHashMap::default(),
             in_type3_glyph: false,
+            image_key: None,
         }
     }
 
@@ -180,6 +290,8 @@
 pub struct RenderCache<'a> {
     pub(crate) interpreter_cache: InterpreterCache<'a>,
     pub(crate) outline_cache: Rc<RefCell<FxHashMap<u128, Rc<BezPath>>>>,
+    /// Tandem patch: whether the last render passed its pixmap budget.
+    pub(crate) over_budget: Rc<std::cell::Cell<bool>>,
 }
 
 impl<'a> RenderCache<'a> {
@@ -188,17 +300,46 @@
         Self {
             interpreter_cache: InterpreterCache::new(),
             outline_cache: Rc::new(RefCell::new(FxHashMap::default())),
+            over_budget: Rc::default(),
         }
     }
+
+    /// Tandem patch: whether the last render with this cache passed
+    /// [`RenderSettings::max_pixmap_pixels`] or
+    /// [`RenderSettings::max_scene_bytes`], or the interpreter's budget for
+    /// sampled function tables: something it would have drawn was left out,
+    /// so the page is incomplete.
+    pub fn over_budget(&self) -> bool {
+        self.over_budget.get() || self.interpreter_cache.over_budget()
+    }
 }
 
 /// Settings for rendering a page.
-#[derive(Clone, Copy, Default)]
+#[derive(Clone, Copy)]
 pub struct RenderSettings {
     /// Whether all images should forcibly be rendered with bilinear interpolation.
     pub force_image_interpolation: bool,
+    /// Tandem patch: the pixels the image, pattern, shading and soft-mask
+    /// pixmaps of one render hold together; a pixmap past it is drawn
+    /// reduced to fit, or left out once too little remains
+    /// ([`RenderCache::over_budget`]).
+    pub max_pixmap_pixels: f64,
+    /// Tandem patch: the estimated bytes vello keeps for the fills, strokes
+    /// and clips of one render; drawing past it is left out
+    /// ([`RenderCache::over_budget`]).
+    pub max_scene_bytes: f64,
 }
 
+impl Default for RenderSettings {
+    fn default() -> Self {
+        Self {
+            force_image_interpolation: false,
+            max_pixmap_pixels: MAX_RENDER_PIXMAP_PIXELS,
+            max_scene_bytes: MAX_RENDER_SCENE_BYTES,
+        }
+    }
+}
+
 /// Settings for the output pixmap.
 #[derive(Clone, Copy)]
 pub struct PixmapSettings {
@@ -289,6 +430,7 @@
     ctx.reset_mask();
     ctx.reset_filter_effect();
 
+    cache.over_budget.set(false);
     let global = GlobalState::new(cache, render_settings);
     let mut device = Renderer::new(ctx, &global);
     let mut clip_path = page.intersected_crop_box().to_kurbo().to_path(0.1);
diff -ru a/src/mask.rs b/src/mask.rs
--- a/src/mask.rs
+++ b/src/mask.rs
@@ -9,7 +9,7 @@
     pub(super) fn apply_soft_mask(&mut self, mask: Option<&SoftMask<'_>>) {
         let settings = *self.ctx.render_settings();
         let global = self.global;
-        let mask = mask.map(|m| {
+        let mask = mask.and_then(|m| {
             let width = self.ctx.width();
             let height = self.ctx.height();
 
@@ -39,7 +39,7 @@
             Some(convert_blend_mode(blend_mode)),
             Some(opacity),
             // TODO: Deduplicate
-            mask.map(|m| {
+            mask.and_then(|m| {
                 let width = self.ctx.width();
                 let height = self.ctx.height();
 
@@ -63,7 +63,13 @@
     width: u16,
     height: u16,
     global: &GlobalState,
-) -> Mask {
+) -> Option<Mask> {
+    // Tandem patch: a mask holds a byte a pixel until the page is composed,
+    // within the render's pixmap budget; once that is spent the mask is left
+    // out (the render is over budget).
+    if !global.pixmap_charge(f64::from(width) * f64::from(height) / 4.0) {
+        return None;
+    }
     let mut ctx = vello_cpu::RenderContext::new_with(width, height, derive_settings(&settings));
     let mut renderer = Renderer::new(&mut ctx, global);
 
@@ -112,5 +118,5 @@
         rendered_mask = Mask::from_parts(map, rendered_mask.width(), rendered_mask.height());
     }
 
-    rendered_mask
+    Some(rendered_mask)
 }
diff -ru a/src/paint.rs b/src/paint.rs
--- a/src/paint.rs
+++ b/src/paint.rs
@@ -2,10 +2,13 @@
 use hayro_interpret::Paint;
 use hayro_interpret::encode::{EncodedShadingType, texture_dimensions};
 use hayro_interpret::gradient::SvgGradientKind;
-use hayro_interpret::pattern::Pattern;
+use hayro_interpret::pattern::{Pattern, TilingPattern};
 use hayro_interpret::util::x_y_advances;
+use hayro_interpret::CacheKey;
 use kurbo::{Affine, BezPath, Rect, Shape};
+use std::hash::{Hash, Hasher};
 use std::sync::Arc;
+use vello_cpu::color::palette::css::TRANSPARENT;
 use vello_cpu::color::{AlphaColor, DynamicColor, Srgb};
 use vello_cpu::peniko::{ColorStop, Gradient, ImageQuality, ImageSampler};
 use vello_cpu::{Image, ImageSource, PaintType, PixelMetadata, Pixmap, peniko};
@@ -34,7 +37,6 @@
                         const NATIVE_GRADIENT_TOLERANCE: f32 = 0.01;
 
                         clip_path = s.shading.clip_path.clone();
-                        let encoded = s.encode();
                         let mut bbox = (*path_transform * path_bbox().to_path(0.0)).bounding_box();
 
                         if is_stroke {
@@ -50,6 +52,8 @@
                             self.ctx.width() as f64,
                             self.ctx.height() as f64,
                         ));
+                        // Tandem patch: meshes sampled within the area drawn.
+                        let encoded = s.encode_within(Some(bbox));
 
                         if let EncodedShadingType::RadialAxial(gradient) = &encoded.shading_type
                             && let Some(native) =
@@ -91,30 +95,65 @@
 
                             PaintType::Gradient(gradient)
                         } else {
-                            let (width, height) = texture_dimensions(bbox, 1.0);
-                            let mut image = Vec::with_capacity(width as usize * height as usize);
-                            let mut may_have_transparency = false;
-                            let (width, height, transform) =
-                                encoded.sample_texture(bbox, 1.0, |sample| {
-                                    let pixel =
-                                        AlphaColor::<Srgb>::new(sample).premultiply().to_rgba8();
-                                    may_have_transparency |= pixel.a != 255;
-                                    image.push(pixel);
-                                });
+                            // Tandem patch: the texture counts against the
+                            // render's pixmap budget (sampled more coarsely to
+                            // fit it, or left out once it is spent), and paints
+                            // of one shading defined by its dictionary (not a
+                            // mesh) over one area share it.
+                            let by_dict =
+                                !matches!(encoded.shading_type, EncodedShadingType::Sampled(_));
+                            let shared = (by_dict && s.transfer_function.is_none()).then(|| {
+                                let mut hasher = rustc_hash::FxHasher::default();
+                                (s.cache_key(), s.opacity.to_bits()).hash(&mut hasher);
+                                [bbox.x0, bbox.y0, bbox.x1, bbox.y1]
+                                    .map(f64::to_bits)
+                                    .hash(&mut hasher);
+                                (u128::from(hasher.finish()), 0, 0, 4)
+                            });
+                            let hit = shared.and_then(|key| self.global.shared.borrow().get(&key).cloned());
+                            let (pixmap, transform) = if let Some((pixmap, _, transform)) = hit {
+                                (pixmap, transform)
+                            } else {
+                                let (width, height) = texture_dimensions(bbox, 1.0);
+                                let Some(scale) = self
+                                    .global
+                                    .pixmap_fit(f64::from(width), f64::from(height))
+                                else {
+                                    self.ctx.set_paint(TRANSPARENT);
+                                    return clip_path;
+                                };
+                                let scale = scale as f32;
+                                let (width, height) = texture_dimensions(bbox, scale);
+                                let mut image = Vec::with_capacity(width as usize * height as usize);
+                                let mut may_have_transparency = false;
+                                let (width, height, transform) =
+                                    encoded.sample_texture(bbox, scale, |sample| {
+                                        let pixel = AlphaColor::<Srgb>::new(sample)
+                                            .premultiply()
+                                            .to_rgba8();
+                                        may_have_transparency |= pixel.a != 255;
+                                        image.push(pixel);
+                                    });
+
+                                let pixmap = Arc::new(Pixmap::from_parts(
+                                    bytemuck::cast_vec(image),
+                                    width as u16,
+                                    height as u16,
+                                    PixelMetadata {
+                                        may_have_transparency,
+                                        ..Default::default()
+                                    },
+                                ));
+                                if let Some(key) = shared {
+                                    let entry = (pixmap.clone(), ImageQuality::Medium, transform);
+                                    self.global.shared.borrow_mut().insert(key, entry);
+                                }
+                                (pixmap, transform)
+                            };
                             paint_transform = path_transform.inverse() * transform;
 
-                            let pixmap = Pixmap::from_parts(
-                                bytemuck::cast_vec(image),
-                                width as u16,
-                                height as u16,
-                                PixelMetadata {
-                                    may_have_transparency,
-                                    ..Default::default()
-                                },
-                            );
-
                             let image = Image {
-                                image: ImageSource::Pixmap(Arc::new(pixmap)),
+                                image: ImageSource::Pixmap(pixmap),
                                 sampler: ImageSampler {
                                     x_extend: peniko::Extend::Repeat,
                                     y_extend: peniko::Extend::Repeat,
@@ -145,18 +184,71 @@
                         xs = xs.max(min_x_scale).min(max_x_scale);
                         ys = ys.max(min_y_scale).min(max_y_scale);
 
+                        // Tandem patch: when the steps leave space between
+                        // cells and one cell alone meets the area drawn, that
+                        // cell is drawn at its own size instead of a step-sized
+                        // pixmap; a repeated step pixmap stays within
+                        // MAX_TILE_PIXELS, MAX_TILE_EDGE and the render's
+                        // pixmap budget.
+                        if t.x_step != 0.0
+                            && t.y_step != 0.0
+                            && (f64::from(t.x_step.abs()) > bbox.width()
+                                || f64::from(t.y_step.abs()) > bbox.height())
+                        {
+                            let mut region =
+                                (*path_transform * path_bbox().to_path(0.1)).bounding_box();
+                            if is_stroke {
+                                let (a1, a2) = x_y_advances(path_transform);
+                                let factor = a1.length().max(a2.length()) * self.ctx.stroke().width;
+                                region = region.inflate(factor, factor);
+                            }
+                            let region = region.inflate(1.0, 1.0).intersect(Rect::new(
+                                0.0,
+                                0.0,
+                                self.ctx.width() as f64,
+                                self.ctx.height() as f64,
+                            ));
+                            if let Some(cell) = visible_cell(&t, region) {
+                                let (paint, transform) =
+                                    self.single_cell(&t, cell, xs, ys, path_transform, is_stroke);
+                                self.ctx.set_paint_transform(transform);
+                                self.ctx.set_paint(paint);
+                                return clip_path;
+                            }
+                        }
+
+                        let fit = (MAX_TILE_PIXELS
+                            / (f64::from(xs * t.x_step).abs() * f64::from(ys * t.y_step).abs()))
+                        .sqrt()
+                        .min(1.0) as f32;
+                        if fit.is_finite() && fit > 0.0 {
+                            xs *= fit;
+                            ys *= fit;
+                        }
+                        let edge_fit =
+                            |s: f32, step: f32| (MAX_TILE_EDGE / (s * step).abs()).min(1.0);
+                        xs *= edge_fit(xs, t.x_step);
+                        ys *= edge_fit(ys, t.y_step);
+                        let Some(budget) = self.global.pixmap_fit(
+                            f64::from(tile_edge(xs * t.x_step)),
+                            f64::from(tile_edge(ys * t.y_step)),
+                        ) else {
+                            self.ctx.set_paint(TRANSPARENT);
+                            return clip_path;
+                        };
+                        xs *= budget as f32;
+                        ys *= budget as f32;
+
                         let x_step = xs * t.x_step;
                         let y_step = ys * t.y_step;
 
-                        let scaled_width = bbox.width() as f32 * xs;
-                        let scaled_height = bbox.height() as f32 * ys;
-                        let pix_width = x_step.abs().round() as u16;
-                        let pix_height = y_step.abs().round() as u16;
+                        let pix_width = tile_edge(x_step);
+                        let pix_height = tile_edge(y_step);
 
                         let mut ctx = self.child_context(pix_width, pix_height);
                         let mut renderer = Renderer::new(&mut ctx, self.global);
                         renderer.inside_pattern = true;
-                        let mut initial_transform = Affine::scale_non_uniform(xs as f64, ys as f64)
+                        let initial_transform = Affine::scale_non_uniform(xs as f64, ys as f64)
                             * Affine::translate((-bbox.x0, -bbox.y0));
                         t.interpret(&mut renderer, initial_transform, is_stroke);
                         let mut pix = Pixmap::new(pix_width, pix_height);
@@ -164,17 +256,8 @@
                         let mut resources = vello_cpu::Resources::default();
                         renderer.ctx.render(&mut pix, &mut resources);
 
-                        // TODO: Fix these
-                        if x_step < 0.0 {
-                            initial_transform *=
-                                Affine::new([-1.0, 0.0, 0.0, 1.0, scaled_width as f64, 0.0]);
-                        }
-
-                        if y_step < 0.0 {
-                            initial_transform *=
-                                Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, scaled_height as f64]);
-                        }
-
+                        // Tandem patch: a negative step places cells as its
+                        // magnitude does (MuPDF); hayro mirrored the cell.
                         paint_transform =
                             path_transform.inverse() * t.matrix * initial_transform.inverse();
 
@@ -198,5 +281,113 @@
         self.ctx.set_paint(paint);
 
         clip_path
+    }
+}
+
+// Tandem patch: bounds on tiling pattern pixmaps.
+
+/// Pixels of transparent margin around a single cell; the pad extend repeats
+/// them outside it.
+const CELL_MARGIN: f64 = 2.0;
+/// The largest step pixmap of a repeating tiling pattern (64 MiB of RGBA).
+const MAX_TILE_PIXELS: f64 = (1u32 << 24) as f64;
+/// The longest edge of a step pixmap, well inside vello's u16 coordinates.
+const MAX_TILE_EDGE: f32 = 16384.0;
+
+/// A step pixmap edge in pixels.
+fn tile_edge(step: f32) -> u16 {
+    let px = step.abs().round();
+    if px.is_finite() {
+        px.clamp(1.0, MAX_TILE_EDGE) as u16
+    } else {
+        1
+    }
+}
+
+/// The offset in pattern space of the only cell of `t` that meets the device
+/// `region`; `Some(None)` when no cell does, `None` when several may.
+fn visible_cell(t: &TilingPattern<'_>, region: Rect) -> Option<Option<(f64, f64)>> {
+    if region.width() <= 0.0 || region.height() <= 0.0 {
+        return Some(None);
+    }
+    let inverse = t.matrix.inverse();
+    if t.matrix.determinant() == 0.0 || !inverse.is_finite() {
+        return None;
+    }
+    let area = (inverse * region.to_path(0.1)).bounding_box();
+    // Cells sit at whole multiples of the steps, whatever their signs.
+    let (sx, sy) = (f64::from(t.x_step.abs()), f64::from(t.y_step.abs()));
+    // Cells i with b0 + i·step < hi and b1 + i·step > lo.
+    let range = |lo: f64, hi: f64, b0: f64, b1: f64, step: f64| {
+        (
+            ((lo - b1) / step).floor() + 1.0,
+            ((hi - b0) / step).ceil() - 1.0,
+        )
+    };
+    let (x0, x1) = range(area.x0, area.x1, t.bbox.x0, t.bbox.x1, sx);
+    let (y0, y1) = range(area.y0, area.y1, t.bbox.y0, t.bbox.y1, sy);
+    if !(x0.is_finite() && x1.is_finite() && y0.is_finite() && y1.is_finite()) {
+        return None;
+    }
+    if x1 < x0 || y1 < y0 {
+        return Some(None);
+    }
+    if x1 > x0 || y1 > y0 {
+        return None;
+    }
+    Some(Some((x0 * sx, y0 * sy)))
+}
+
+impl Renderer<'_> {
+    /// One cell of `t` at `cell`, as a padded image of its bounding box.
+    fn single_cell(
+        &self,
+        t: &TilingPattern<'_>,
+        cell: Option<(f64, f64)>,
+        xs: f32,
+        ys: f32,
+        path_transform: &Affine,
+        is_stroke: bool,
+    ) -> (PaintType, Affine) {
+        let Some((dx, dy)) = cell else {
+            return (TRANSPARENT.into(), Affine::IDENTITY);
+        };
+        let bbox = t.bbox;
+        let (mut xs, mut ys) = (f64::from(xs), f64::from(ys));
+        let Some(budget) = self.global.pixmap_fit(
+            (bbox.width() * xs).abs() + 2.0 * CELL_MARGIN,
+            (bbox.height() * ys).abs() + 2.0 * CELL_MARGIN,
+        ) else {
+            return (TRANSPARENT.into(), Affine::IDENTITY);
+        };
+        xs *= budget;
+        ys *= budget;
+        let width = tile_edge((bbox.width() * xs + 2.0 * CELL_MARGIN).ceil() as f32);
+        let height = tile_edge((bbox.height() * ys + 2.0 * CELL_MARGIN).ceil() as f32);
+        let content = Affine::translate((CELL_MARGIN, CELL_MARGIN))
+            * Affine::scale_non_uniform(xs, ys)
+            * Affine::translate((-bbox.x0, -bbox.y0));
+
+        let mut ctx = self.child_context(width, height);
+        let mut renderer = Renderer::new(&mut ctx, self.global);
+        renderer.inside_pattern = true;
+        t.interpret(&mut renderer, content, is_stroke);
+        let mut pix = Pixmap::new(width, height);
+        renderer.ctx.flush();
+        let mut resources = vello_cpu::Resources::default();
+        renderer.ctx.render(&mut pix, &mut resources);
+
+        let transform =
+            path_transform.inverse() * t.matrix * Affine::translate((dx, dy)) * content.inverse();
+        let image = Image {
+            image: ImageSource::Pixmap(Arc::new(pix)),
+            sampler: ImageSampler {
+                x_extend: peniko::Extend::Pad,
+                y_extend: peniko::Extend::Pad,
+                quality: ImageQuality::Medium,
+                alpha: 1.0,
+            },
+        };
+        (PaintType::Image(image), transform)
     }
 }
diff -ru a/src/path.rs b/src/path.rs
--- a/src/path.rs
+++ b/src/path.rs
@@ -1,7 +1,111 @@
 use crate::{Renderer, convert_fill_rule};
 use hayro_interpret::{DrawMode, DrawProps, FillRule, StrokeProps};
-use kurbo::{Affine, BezPath, Point, Rect, Shape};
+use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape};
 
+// Tandem patch: what one path costs vello at once. It flattens the path into
+// lines, sorts the tiles each line crosses and, for a stroke, first expands
+// the outline kurbo strokes (a join or cap at every segment, every dash a
+// subpath): a 50-byte dash pattern on a page-wide line took 1.4 GB natively,
+// and a 24 KB page stroking 2,000,000 tiny segments 2.3 GB of WASM memory
+// (8,000,000 aborted the engine). Estimated per segment, per further line a
+// curve flattens into (about the square root of its device length) and per
+// device pixel of length, as measured on the WASM build (vello_cpu 0.3):
+// about 140 bytes a segment of a fill and 1,240 of a stroke (800,000 and
+// 400,000 tiny segments), natively 1.5 and 3 to 4 bytes a pixel (20,000
+// page-sized curves at 72 and 288 dpi), rounded up. vello culls what lies
+// outside the pixmap while it flattens, so lines and pixels count only
+// within it (a line or curve 10^24 units long costs what its visible part
+// does); kurbo dashes the whole path first, so dashes count along all of
+// it. A stroke's outline runs along both sides of each segment and around
+// its join, about three widths a segment more (400,000 tiny segments
+// stroked 1,000 pixels wide would cross tiles for gigabytes).
+/// The estimated bytes one fill, stroke or clip may cost; past it the
+/// render is over budget.
+pub const MAX_PATH_BYTES: f64 = (1u64 << 29) as f64;
+const FILL_SEGMENT_BYTES: f64 = 160.0;
+const FILL_LINE_BYTES: f64 = 24.0;
+const FILL_PIXEL_BYTES: f64 = 2.0;
+const STROKE_SEGMENT_BYTES: f64 = 1280.0;
+const STROKE_LINE_BYTES: f64 = 64.0;
+const STROKE_PIXEL_BYTES: f64 = 4.0;
+/// Segments a dash adds to a stroke's outline: its own and its caps.
+const DASH_SEGMENTS: f64 = 2.0;
+/// Widths of outline a stroked segment adds (its sides and its join).
+const STROKE_WIDTHS: f64 = 3.0;
+
+/// What a path costs to flatten and dash, in device pixels under a
+/// transform, from its control polygons (upper bounds).
+struct Extent {
+    segments: f64,
+    /// Segments with a part within the pixmap.
+    near: f64,
+    /// Further lines its curves flatten into, within the pixmap.
+    lines: f64,
+    /// Its length within the pixmap.
+    visible: f64,
+    /// Its whole length.
+    length: f64,
+}
+
+/// Tandem patch: the extent of `path` under `transform` in a pixmap of
+/// `area`.
+fn path_extent(path: &BezPath, transform: &Affine, area: Rect) -> Extent {
+    let (mut segments, mut near, mut lines, mut visible, mut length) = (0.0, 0.0, 0.0, 0.0, 0.0);
+    let mut last = Point::ZERO;
+    let mut start = Point::ZERO;
+    // A segment's whole length and the most of it inside `area` (the
+    // diagonal of its control points' box there).
+    let polygon = |points: &[Point], from: Point| {
+        let mut previous = *transform * from;
+        let mut bounds = Rect::from_points(previous, previous);
+        let mut total = 0.0;
+        for point in points {
+            let point = *transform * *point;
+            total += (point - previous).hypot();
+            bounds = bounds.union_pt(point);
+            previous = point;
+        }
+        let inside = bounds.intersect(area);
+        let seen = if inside.width() < 0.0 || inside.height() < 0.0 {
+            0.0
+        } else {
+            inside.width().hypot(inside.height())
+        };
+        (total, seen.min(total))
+    };
+    for element in path.elements() {
+        let (points, curve): (&[Point], bool) = match element {
+            PathEl::MoveTo(p) => {
+                (last, start) = (*p, *p);
+                segments += 1.0;
+                continue;
+            }
+            PathEl::ClosePath => (std::slice::from_ref(&start), false),
+            PathEl::LineTo(p) => (std::slice::from_ref(p), false),
+            PathEl::QuadTo(a, b) => (&[*a, *b][..], true),
+            PathEl::CurveTo(a, b, c) => (&[*a, *b, *c][..], true),
+        };
+        let (reach, seen) = polygon(points, last);
+        segments += 1.0;
+        if seen > 0.0 {
+            near += 1.0;
+        }
+        if curve {
+            lines += seen.sqrt();
+        }
+        visible += seen;
+        length += reach;
+        last = points.last().copied().unwrap_or(last);
+    }
+    Extent {
+        segments,
+        near,
+        lines,
+        visible,
+        length,
+    }
+}
+
 impl Renderer<'_> {
     pub(super) fn set_stroke_properties(&mut self, stroke_props: &StrokeProps, is_text: bool) {
         let threshold = if is_text { 0.25 } else { 1.0 };
@@ -28,10 +132,62 @@
             dash_pattern: stroke_props.dash_array.iter().map(|n| *n as f64).collect(),
             dash_offset: stroke_props.dash_offset as f64,
         };
+        // Tandem patch: a dash pattern shorter than half a pixel (or 0.01
+        // units) strokes solid, as MuPDF 1.28.1 does; the dashes kurbo
+        // expanded for it cost memory for nothing visible.
+        let period: f64 = stroke.dash_pattern.iter().sum();
+        let stroke = if !stroke.dash_pattern.is_empty()
+            && (period < 0.01 || period * f64::from(min_factor) < 0.5)
+        {
+            kurbo::Stroke {
+                dash_pattern: Default::default(),
+                ..stroke
+            }
+        } else {
+            stroke
+        };
 
         self.ctx.set_stroke(stroke);
     }
 
+    /// Tandem patch: whether drawing `path` (stroked with the current stroke
+    /// when `stroke`) fits [`MAX_PATH_BYTES`]; else the render is over
+    /// budget.
+    pub(crate) fn path_fits(&self, path: &BezPath, stroke: bool) -> bool {
+        let transform = *self.ctx.transform();
+        let (width, height) = (f64::from(self.ctx.width()), f64::from(self.ctx.height()));
+        let area = Rect::new(0.0, 0.0, width, height);
+        let bytes = if stroke {
+            // Segments whose outline reaches into the pixmap count.
+            let margin = self.stroke_margin().min(width + height);
+            let extent = path_extent(path, &transform, area.inflate(margin, margin));
+            let style = self.ctx.stroke();
+            let period: f64 = style.dash_pattern.iter().sum();
+            let scale = f64::from(max_factor(&transform));
+            let dashes = if style.dash_pattern.is_empty() || period <= 0.0 {
+                0.0
+            } else {
+                extent.length / (period * scale) * style.dash_pattern.len() as f64
+            };
+            let outline = (style.width * scale * STROKE_WIDTHS).min(2.0 * (width + height));
+            (extent.segments + dashes * DASH_SEGMENTS) * STROKE_SEGMENT_BYTES
+                + extent.lines * STROKE_LINE_BYTES
+                + (extent.visible + extent.near * outline) * STROKE_PIXEL_BYTES
+        } else {
+            let extent = path_extent(path, &transform, area);
+            extent.segments * FILL_SEGMENT_BYTES
+                + extent.lines * FILL_LINE_BYTES
+                + extent.visible * FILL_PIXEL_BYTES
+        };
+        // Non-finite geometry counts as past the budget.
+        if bytes <= MAX_PATH_BYTES {
+            true
+        } else {
+            self.global.over_budget.set(true);
+            false
+        }
+    }
+
     pub(super) fn stroke_path(
         &mut self,
         path: &BezPath,
@@ -41,6 +197,12 @@
     ) {
         self.apply_draw_props(&props);
         self.set_stroke_properties(stroke_props, is_text);
+        // Tandem patch: within the render's scene and path budgets.
+        if !self.scene_fits(path.bounding_box(), self.stroke_margin())
+            || !self.path_fits(path, true)
+        {
+            return;
+        }
 
         let clip_path = self.set_paint(&props.paint, || path.bounding_box(), true);
         if let Some(clip_path) = clip_path.as_ref() {
@@ -55,8 +217,13 @@
     pub(super) fn fill_path(&mut self, path: &BezPath, props: DrawProps<'_>, fill_rule: FillRule) {
         self.ctx.set_fill_rule(convert_fill_rule(fill_rule));
         self.apply_draw_props(&props);
+        // Tandem patch: within the render's scene and path budgets.
+        let bbox = path.bounding_box();
+        if !self.scene_fits(bbox, 0.0) || !self.path_fits(path, false) {
+            return;
+        }
 
-        let clip_path = self.set_paint(&props.paint, || path.bounding_box(), false);
+        let clip_path = self.set_paint(&props.paint, || bbox, false);
         if let Some(clip_path) = clip_path.as_ref() {
             self.push_clip_path_inner(clip_path, fill_rule);
         }
@@ -99,6 +266,10 @@
             DrawMode::Fill(fill_rule) => {
                 self.ctx.set_fill_rule(convert_fill_rule(*fill_rule));
                 self.apply_draw_props(&props);
+                // Tandem patch: within the render's scene budget.
+                if !self.scene_fits(*rect, 0.0) {
+                    return;
+                }
 
                 let clip_path = self.set_paint(&props.paint, || *rect, false);
                 if let Some(clip_path) = clip_path.as_ref() {
@@ -122,6 +293,16 @@
             }
             DrawMode::Invisible => {}
         }
+    }
+}
+
+impl Renderer<'_> {
+    /// Tandem patch: how far the current stroke reaches past its path, in
+    /// pixels.
+    pub(crate) fn stroke_margin(&self) -> f64 {
+        let stroke = self.ctx.stroke();
+        let reach = stroke.width * stroke.miter_limit.max(1.0);
+        reach * f64::from(max_factor(self.ctx.transform()))
     }
 }
 
```
