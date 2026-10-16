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
- `src/image.rs`, images:
  - An image pixmap holds at most `MAX_IMAGE_PIXELS` (2^25, 128 MiB of RGBA)
    and 32,767 pixels a side, and counts against the render's budget; a
    larger one is drawn reduced. hayro cut a width past 65,535 to 16 bits and
    vello refused the pixmap.
  - A reduction of more than 8× starts with a whole-number box filter
    (`box_reduce`), so the resampler's work follows its output, not the
    samples (pdf.js `issue16263.pdf`: 18 s → 0.4 s with the decoder's own
    reduction in hayro-interpret).
  - A resampler failure leaves the resized samples zero instead of
    panicking (`unwrap` on `pic_scale` results).
- `src/lib.rs`, the render's pixmap budget: vello keeps every image and
  pattern pixmap a page paints until the page is composed, so a small file
  drawing one large image many times held gigabytes. `GlobalState` counts
  their pixels; past `RenderSettings::max_pixmap_pixels` (default
  `MAX_RENDER_PIXMAP_PIXELS`, 2^27, 512 MiB of RGBA) each further pixmap is
  drawn reduced to at most 4,096 pixels. `RenderSettings` gains that field
  and a `Default` impl in place of the derive.

## Diff against the published crate

```diff
diff -ru a/src/image.rs b/src/image.rs
--- a/src/image.rs
+++ b/src/image.rs
@@ -19,6 +19,67 @@
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
+    if (kx == 1 && ky == 1) || w == 0 || h == 0 || data.len() < w * h * N {
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
@@ -175,16 +236,30 @@
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
 
@@ -217,8 +292,8 @@
 
         let has_alpha = alpha_data.is_some();
         let mut may_have_transparency = has_alpha;
-        let needs_resize = x_scale < 1.0 || y_scale < 1.0;
-        let (new_width, new_height) = if needs_resize {
+        let mut needs_resize = x_scale < 1.0 || y_scale < 1.0;
+        let (mut new_width, mut new_height) = if needs_resize {
             let w = (img_width as f32 * x_scale)
                 .ceil()
                 .max(1.0)
@@ -234,6 +309,23 @@
         } else {
             (img_width, img_height)
         };
+        // Tandem patch: an image pixmap holds at most MAX_IMAGE_PIXELS and
+        // u16::MAX / 2 pixels a side, within the render's pixmap budget; a
+        // larger one is drawn reduced.
+        let fit = (MAX_IMAGE_PIXELS / (f64::from(new_width) * f64::from(new_height)))
+            .sqrt()
+            .min(f64::from(u16::MAX / 2) / f64::from(new_width))
+            .min(f64::from(u16::MAX / 2) / f64::from(new_height))
+            .min(1.0);
+        let fit = fit
+            * self
+                .global
+                .pixmap_fit(f64::from(new_width) * fit, f64::from(new_height) * fit);
+        if fit < 1.0 {
+            new_width = ((f64::from(new_width) * fit) as u32).max(1);
+            new_height = ((f64::from(new_height) * fit) as u32).max(1);
+            needs_resize = true;
+        }
 
         // For luma images without alpha, we can resize as single-channel and
         // expand to RGBA afterwards, which is ~4x faster.
diff -ru a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -73,8 +73,20 @@
     outline_cache: Rc<RefCell<FxHashMap<u128, Rc<BezPath>>>>,
     scaler: Scaler,
     force_image_interpolation: bool,
+    /// Tandem patch: pixels of the image and pattern pixmaps made so far,
+    /// and the most they may hold.
+    pixmap_pixels: std::cell::Cell<f64>,
+    pixmap_budget: f64,
 }
 
+// Tandem patch: vello keeps every image and pattern pixmap a page paints
+// until the page is composed, so their sum is bounded per render.
+/// The pixels a render's image and pattern pixmaps hold together by default
+/// (512 MiB of RGBA).
+pub const MAX_RENDER_PIXMAP_PIXELS: f64 = (1u32 << 27) as f64;
+/// The pixels a pixmap may keep once that budget is spent.
+const MIN_PIXMAP_PIXELS: f64 = 4096.0;
+
 impl GlobalState {
     pub(crate) fn new(cache: &RenderCache<'_>, settings: &RenderSettings) -> Self {
         Self {
@@ -82,8 +94,26 @@
             outline_cache: cache.outline_cache.clone(),
             scaler: Scaler::new(image::RESAMPLING_FUNCTION),
             force_image_interpolation: settings.force_image_interpolation,
+            pixmap_pixels: std::cell::Cell::new(0.0),
+            pixmap_budget: settings.max_pixmap_pixels,
         }
     }
+
+    /// Tandem patch: the scale, at most 1, that keeps a `width` × `height`
+    /// pixmap within what remains of the pixmap budget (or
+    /// MIN_PIXMAP_PIXELS), whose pixels are then counted.
+    pub(crate) fn pixmap_fit(&self, width: f64, height: f64) -> f64 {
+        let used = self.pixmap_pixels.get();
+        let wanted = (width * height).max(1.0);
+        let allowed = (self.pixmap_budget - used).max(MIN_PIXMAP_PIXELS);
+        let fit = if wanted.is_finite() {
+            (allowed / wanted).sqrt().min(1.0)
+        } else {
+            0.0
+        };
+        self.pixmap_pixels.set(used + wanted.min(allowed));
+        fit
+    }
 }
 
 pub(crate) struct Renderer<'a> {
@@ -193,10 +223,22 @@
 }
 
 /// Settings for rendering a page.
-#[derive(Clone, Copy, Default)]
+#[derive(Clone, Copy)]
 pub struct RenderSettings {
     /// Whether all images should forcibly be rendered with bilinear interpolation.
     pub force_image_interpolation: bool,
+    /// Tandem patch: the pixels the image and pattern pixmaps of one render
+    /// hold together; past it they are drawn reduced.
+    pub max_pixmap_pixels: f64,
+}
+
+impl Default for RenderSettings {
+    fn default() -> Self {
+        Self {
+            force_image_interpolation: false,
+            max_pixmap_pixels: MAX_RENDER_PIXMAP_PIXELS,
+        }
+    }
 }
 
 /// Settings for the output pixmap.
diff -ru a/src/paint.rs b/src/paint.rs
--- a/src/paint.rs
+++ b/src/paint.rs
@@ -2,10 +2,11 @@
 use hayro_interpret::Paint;
 use hayro_interpret::encode::{EncodedShadingType, texture_dimensions};
 use hayro_interpret::gradient::SvgGradientKind;
-use hayro_interpret::pattern::Pattern;
+use hayro_interpret::pattern::{Pattern, TilingPattern};
 use hayro_interpret::util::x_y_advances;
 use kurbo::{Affine, BezPath, Rect, Shape};
 use std::sync::Arc;
+use vello_cpu::color::palette::css::TRANSPARENT;
 use vello_cpu::color::{AlphaColor, DynamicColor, Srgb};
 use vello_cpu::peniko::{ColorStop, Gradient, ImageQuality, ImageSampler};
 use vello_cpu::{Image, ImageSource, PaintType, PixelMetadata, Pixmap, peniko};
@@ -34,7 +35,6 @@
                         const NATIVE_GRADIENT_TOLERANCE: f32 = 0.01;
 
                         clip_path = s.shading.clip_path.clone();
-                        let encoded = s.encode();
                         let mut bbox = (*path_transform * path_bbox().to_path(0.0)).bounding_box();
 
                         if is_stroke {
@@ -50,6 +50,8 @@
                             self.ctx.width() as f64,
                             self.ctx.height() as f64,
                         ));
+                        // Tandem patch: meshes sampled within the area drawn.
+                        let encoded = s.encode_within(Some(bbox));
 
                         if let EncodedShadingType::RadialAxial(gradient) = &encoded.shading_type
                             && let Some(native) =
@@ -145,18 +147,68 @@
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
+                        let budget = self.global.pixmap_fit(
+                            f64::from(tile_edge(xs * t.x_step)),
+                            f64::from(tile_edge(ys * t.y_step)),
+                        ) as f32;
+                        xs *= budget;
+                        ys *= budget;
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
@@ -164,17 +216,8 @@
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
 
@@ -198,5 +241,111 @@
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
+        let budget = self.global.pixmap_fit(
+            (bbox.width() * xs).abs() + 2.0 * CELL_MARGIN,
+            (bbox.height() * ys).abs() + 2.0 * CELL_MARGIN,
+        );
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
```
