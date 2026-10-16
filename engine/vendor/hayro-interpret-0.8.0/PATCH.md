# hayro-interpret 0.8.0, patched for Tandem PDF

This directory is hayro-interpret 0.8.0 as published on crates.io (`.crate`
checksum `ac9df074ff163a63864cec9b0bf640ccbff8abf4e44481df7369ce0ac172ffbe`,
the one `Cargo.lock` pinned before), with the patch below. It is wired in
with `[patch.crates-io]` in `engine/Cargo.toml` and `engine/bridge/Cargo.toml`.
Licence: Apache-2.0 OR MIT (`LICENSE-APACHE`, `LICENSE-MIT`, unchanged); the
bundled assets keep their notices (`assets/`, `engine/NOTICE`). Why a patch
and not a guard in the engine: a tint transform's output count can depend on
its input (`{dup 0.5 gt {0} {0 0 0 0} ifelse}`), so no rewrite of the PDF
fixes it, and the other changes sit inside hayro's decoders. Every change is
marked `Tandem patch` in the source. Design notes and measurements:
`engine/design/app-corpus-robust.md`; tests: `engine/tests/view_robust.rs`.

## Changes

- `src/color/separation.rs`, `src/color/device_n.rs`: the tint transform's
  values are cut or padded with 0 to the alternate space's component count
  before encoding. hayro passed them on, and `DeviceRgb::convert` panicked
  copying 1,024 bytes into 768 (`device_rgb.rs:8`; the seven veraPDF
  6-2-4-4-t01 files). `src/color/device_rgb.rs` returns a conversion failure
  instead of panicking when the lengths still differ. A DeviceN pixel equal
  to the one before reuses its values, so flat areas and the zero padding of
  short image data cost one tint evaluation (a 25-byte image declaring 64 ×
  100,000 samples in a three-colorant space took 3 s).
- `src/function/type4.rs`: a PostScript function's outputs are the top n
  values of its stack, n from /Range, 0 for those the program did not leave
  (MuPDF 1.28.1 and pdf.js take the top values); hayro returned the whole
  stack.
- `src/shading.rs`: a lattice mesh with fewer than two vertices per row (or
  none given) is read with two (MuPDF warns and reads two); zero read empty
  rows without end, allocating until the process died (found by the
  generated cases of `tests/view_robust.rs`). Meshes keep at most
  `MAX_MESH_TRIANGLES` (2^18) triangles, a lattice at most half as many
  vertices, and patch meshes `MAX_MESH_PATCHES` (2^14) patches; the rest is
  not drawn. A mesh is read whole when the shading loads, a few bits per
  triangle or patch, so a small stream could ask for gigabytes.
- `src/encode.rs`: Coons and tensor-product patches are triangulated and
  sampled one at a time, not all collected first, on a grid of a point every
  4 device pixels of the patch's extent (2 to hayro's 20 a side, so up to 722
  triangles; `patch_grid`, `to_triangles_in`). Meshes are sampled only inside
  the area drawn (`ShadingPattern::encode_within`, which hayro's painter
  calls); a triangle far off the page was sampled over up to 65,535 × 65,535
  pixels. One mesh paint tests at most `MAX_SAMPLE_WORK` (2^26) pixels,
  counting one per triangle too; the rest of the mesh is not drawn (2^18
  page-sized triangles took 40 s).
- `src/x_object/decode/mod.rs`, `image.rs`, `mask.rs`, images and masks:
  - An image of 1 to 7 or 9 to 15 bits per component is unpacked into its
    declared size, missing data padded, so a few bytes declaring 100,000 ×
    100,000 samples asked for 10 GB; past `MAX_UNPACKED_SAMPLES` (2^28) it
    is not drawn. A bilevel mask reserves memory for the rows its data holds.
  - An image or mask drawn more than 8× smaller than its samples is reduced
    by whole-number box factors when decoded (`reduction`, `box_reduce`); a
    one-bit mask is counted from its bits without expanding them
    (`decode_mask_data_reduced`).
  - A decoded image or reduced mask is kept for the rest of the render
    (`Cache::get_or_insert_sized`, within `REDUCED_CACHE_BYTES`, 32 MiB), so
    an image drawn many times is decoded once (pdf.js `issue16263.pdf` draws
    one 34,862 × 4,332 mask many times), unless its decoding depends on the
    drawing state (a transfer function, a colour space named in the
    resources).

## Diff against the published crate

```diff
diff -ru a/src/cache.rs b/src/cache.rs
--- a/src/cache.rs
+++ b/src/cache.rs
@@ -50,6 +50,49 @@
     }
 }
 
+// Tandem patch: values kept within a byte budget.
+/// The key holding the bytes kept by `get_or_insert_sized`.
+const SIZED_TOTAL: u128 = u128::MAX;
+
+impl Cache {
+    /// The value kept for `id`, or `f()`'s, which is kept only while the
+    /// values kept this way take at most `budget` bytes (by `size`).
+    pub(crate) fn get_or_insert_sized<T: Clone + Send + Sync + 'static>(
+        &self,
+        id: u128,
+        budget: usize,
+        size: impl FnOnce(&T) -> usize,
+        f: impl FnOnce() -> Option<T>,
+    ) -> Option<T> {
+        let lock = || {
+            self.0
+                .lock()
+                .unwrap_or_else(std::sync::PoisonError::into_inner)
+        };
+        if let Some(Some(value)) = lock().get(&id)
+            && let Some(value) = value.downcast_ref::<T>()
+        {
+            return Some(value.clone());
+        }
+        let value = f()?;
+        let bytes = size(&value);
+        let mut locked = lock();
+        let total = locked
+            .entry(SIZED_TOTAL)
+            .or_insert_with(|| Some(Box::new(0usize)))
+            .as_mut()
+            .and_then(|total| total.downcast_mut::<usize>())
+            .filter(|total| total.saturating_add(bytes) <= budget)
+            .map(|total| {
+                *total += bytes;
+            });
+        if total.is_some() {
+            locked.insert(id, Some(Box::new(value.clone())));
+        }
+        Some(value)
+    }
+}
+
 /// A trait for objects that can generate a unique cache key.
 pub trait CacheKey {
     /// Returns the cache key for this object.
diff -ru a/src/color/device_n.rs b/src/color/device_n.rs
--- a/src/color/device_n.rs
+++ b/src/color/device_n.rs
@@ -41,17 +41,33 @@
     }
 
     fn evaluate(&self, input: &[u8]) -> Vec<u8> {
-        input
-            .chunks_exact(self.num_components as usize)
-            .flat_map(|n| {
-                let input = n.iter().map(|value| *value as f32 / 255.0).collect();
-                let values = self
-                    .tint_transform
-                    .eval(input)
-                    .unwrap_or(self.alternate_space.initial_color());
-                self.alternate_space.encode_values(&values)
-            })
-            .collect()
+        // Tandem patch: a pixel equal to the one before reuses its values, so
+        // flat areas (and the padding of short image data) cost one
+        // evaluation.
+        let mut out = Vec::new();
+        let mut last: Option<(&[u8], usize)> = None;
+        for n in input.chunks_exact(self.num_components as usize) {
+            if let Some((previous, at)) = last
+                && previous == n
+            {
+                let start = out.len();
+                out.extend_from_within(at..start);
+                last = Some((n, start));
+                continue;
+            }
+            let at = out.len();
+            let input = n.iter().map(|value| *value as f32 / 255.0).collect();
+            let mut values = self
+                .tint_transform
+                .eval(input)
+                .unwrap_or(self.alternate_space.initial_color());
+            // Tandem patch: as many values as the alternate space has
+            // components, cut or padded with 0 (MuPDF).
+            values.resize(self.alternate_space.num_components() as usize, 0.0);
+            out.extend_from_slice(&self.alternate_space.encode_values(&values));
+            last = Some((n, at));
+        }
+        out
     }
 
     fn convert_inner(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
diff -ru a/src/color/device_rgb.rs b/src/color/device_rgb.rs
--- a/src/color/device_rgb.rs
+++ b/src/color/device_rgb.rs
@@ -5,6 +5,11 @@
 
 impl ToRgb for DeviceRgb {
     fn convert(&self, input: &[u8], output: &mut [u8]) -> Option<()> {
+        // Tandem patch: a conversion failure instead of a panic when the
+        // lengths differ.
+        if input.len() != output.len() {
+            return None;
+        }
         output.copy_from_slice(input);
 
         Some(())
diff -ru a/src/color/separation.rs b/src/color/separation.rs
--- a/src/color/separation.rs
+++ b/src/color/separation.rs
@@ -36,10 +36,13 @@
         let evaluated = input
             .iter()
             .flat_map(|n| {
-                let values = self
+                let mut values = self
                     .tint_transform
                     .eval(smallvec![*n as f32 / 255.0])
                     .unwrap_or(self.alternate_space.initial_color());
+                // Tandem patch: as many values as the alternate space has
+                // components, cut or padded with 0 (MuPDF).
+                values.resize(self.alternate_space.num_components() as usize, 0.0);
                 self.alternate_space.encode_values(&values)
             })
             .collect::<Vec<u8>>();
diff -ru a/src/encode.rs b/src/encode.rs
--- a/src/encode.rs
+++ b/src/encode.rs
@@ -3,7 +3,7 @@
 use crate::color::{AlphaColor, ColorComponents, ColorSpace};
 use crate::interpret::state::ActiveTransferFunction;
 use crate::pattern::ShadingPattern;
-use crate::shading::{ShadingFunction, ShadingType, Triangle};
+use crate::shading::{GRID_SIZE, ShadingFunction, ShadingType, Triangle};
 use crate::util::x_y_advances;
 use kurbo::{Affine, Point, Rect};
 use rustc_hash::FxHashMap;
@@ -187,6 +187,12 @@
 impl ShadingPattern {
     /// Encode the shading pattern.
     pub fn encode(&self) -> EncodedShadingPattern {
+        self.encode_within(None)
+    }
+
+    /// Tandem patch: [`Self::encode`] with meshes sampled only where `area`
+    /// (device pixels) may look them up, a patch at a time.
+    pub fn encode_within(&self, area: Option<Rect>) -> EncodedShadingPattern {
         let base_transform;
 
         let shading_type = match self.shading.shading_type.as_ref() {
@@ -217,7 +223,9 @@
                 function,
             } => {
                 let full_transform = self.matrix;
-                let samples = sample_triangles(triangles, full_transform);
+                let mut samples = FxHashMap::default();
+                let mut work = 0.0;
+                sample_triangles(&mut samples, triangles, full_transform, area, &mut work);
 
                 base_transform = Affine::IDENTITY;
 
@@ -227,14 +235,20 @@
                 })
             }
             ShadingType::CoonsPatchMesh { patches, function } => {
+                let full_transform = self.matrix;
+                let mut samples = FxHashMap::default();
                 let mut triangles = vec![];
+                let mut work = 0.0;
                 for patch in patches {
-                    patch.to_triangles(&mut triangles);
+                    triangles.clear();
+                    let grid = patch_grid(&patch.control_points, full_transform);
+                    patch.to_triangles_in(&mut triangles, grid);
+                    if !sample_triangles(&mut samples, &triangles, full_transform, area, &mut work)
+                    {
+                        break;
+                    }
                 }
 
-                let full_transform = self.matrix;
-                let samples = sample_triangles(&triangles, full_transform);
-
                 base_transform = Affine::IDENTITY;
 
                 EncodedShadingType::Sampled(EncodedSampledShading {
@@ -243,14 +257,20 @@
                 })
             }
             ShadingType::TensorProductPatchMesh { patches, function } => {
+                let full_transform = self.matrix;
+                let mut samples = FxHashMap::default();
                 let mut triangles = vec![];
+                let mut work = 0.0;
                 for patch in patches {
-                    patch.to_triangles(&mut triangles);
+                    triangles.clear();
+                    let grid = patch_grid(&patch.control_points, full_transform);
+                    patch.to_triangles_in(&mut triangles, grid);
+                    if !sample_triangles(&mut samples, &triangles, full_transform, area, &mut work)
+                    {
+                        break;
+                    }
                 }
 
-                let full_transform = self.matrix;
-                let samples = sample_triangles(&triangles, full_transform);
-
                 base_transform = Affine::IDENTITY;
 
                 EncodedShadingType::Sampled(EncodedSampledShading {
@@ -329,11 +349,48 @@
     )
 }
 
+/// Tandem patch: the grid a patch is triangulated on, a point every 4 device
+/// pixels of its control points' extent (2 to GRID_SIZE a side): a patch a
+/// few pixels wide needs no 722 triangles.
+fn patch_grid(points: &[Point], transform: Affine) -> usize {
+    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
+    for p in points {
+        let p = transform * *p;
+        x0 = x0.min(p.x);
+        y0 = y0.min(p.y);
+        x1 = x1.max(p.x);
+        y1 = y1.max(p.y);
+    }
+    let extent = (x1 - x0).max(y1 - y0);
+    if extent.is_finite() {
+        (extent / 4.0).ceil().clamp(1.0, (GRID_SIZE - 1) as f64) as usize + 1
+    } else {
+        GRID_SIZE
+    }
+}
+
+/// Tandem patch: the pixels one mesh may test when sampled, half a second's
+/// work; past it the rest of the mesh is not drawn.
+const MAX_SAMPLE_WORK: f64 = (1u64 << 26) as f64;
+
+/// Samples `triangles` into `map`; false once `work` (pixels tested, and one
+/// per triangle) passes MAX_SAMPLE_WORK.
 fn sample_triangles(
+    map: &mut FxHashMap<(u16, u16), ColorComponents>,
     triangles: &[Triangle],
     transform: Affine,
-) -> FxHashMap<(u16, u16), ColorComponents> {
-    let mut map = FxHashMap::default();
+    area: Option<Rect>,
+    work: &mut f64,
+) -> bool {
+    // Tandem patch: only the pixels `area` may look up, with a pixel's margin.
+    let clip = area.map_or(Rect::new(0.0, 0.0, 65535.0, 65535.0), |a| {
+        Rect::new(
+            a.x0.floor() - 1.0,
+            a.y0.floor() - 1.0,
+            a.x1.ceil() + 1.0,
+            a.y1.ceil() + 1.0,
+        )
+    });
 
     for t in triangles {
         let t = {
@@ -351,7 +408,13 @@
             Triangle::new(v0, v1, v2)
         };
 
-        let bbox = t.bounding_box();
+        let bbox = t.bounding_box().intersect(clip);
+        *work += 1.0
+            + (bbox.x1.ceil() - bbox.x0.floor()).max(0.0)
+                * (bbox.y1.ceil() - bbox.y0.floor()).max(0.0);
+        if *work > MAX_SAMPLE_WORK {
+            return false;
+        }
 
         for y in (bbox.y0.floor() as u16)..(bbox.y1.ceil() as u16) {
             for x in (bbox.x0.floor() as u16)..(bbox.x1.ceil() as u16) {
@@ -362,8 +425,7 @@
             }
         }
     }
-
-    map
+    true
 }
 
 fn encode_function_shading(domain: &[f32; 4], function: &ShadingFunction) -> EncodedShadingType {
diff -ru a/src/function/type4.rs b/src/function/type4.rs
--- a/src/function/type4.rs
+++ b/src/function/type4.rs
@@ -39,7 +39,19 @@
 
         eval_inner(&self.program, &mut arg_stack)?;
 
-        let mut out: SmallVec<_> = arg_stack.items().iter().map(|i| i.as_f32()).collect();
+        // Tandem patch: the outputs are the top n values of the stack, n from
+        // /Range, 0 for those the program did not leave (MuPDF, pdf.js).
+        let items = arg_stack.items();
+        let mut out: SmallVec<_> = match self.clamper.range.as_ref().map(|r| r.len()) {
+            Some(n) => std::iter::repeat_n(0.0, n.saturating_sub(items.len()))
+                .chain(
+                    items[items.len().saturating_sub(n)..]
+                        .iter()
+                        .map(|i| i.as_f32()),
+                )
+                .collect(),
+            None => items.iter().map(|i| i.as_f32()).collect(),
+        };
 
         self.clamper.clamp_output(&mut out);
 
diff -ru a/src/shading.rs b/src/shading.rs
--- a/src/shading.rs
+++ b/src/shading.rs
@@ -21,6 +21,15 @@
 use smallvec::{SmallVec, smallvec};
 use std::sync::Arc;
 
+// Tandem patch: mesh budgets. A mesh is read whole when the shading loads, a
+// few bits of data per triangle or patch, so a small stream can ask for
+// gigabytes; past these the rest of the mesh is not drawn.
+/// The most triangles a free-form or lattice mesh keeps (about 60 MB).
+pub(crate) const MAX_MESH_TRIANGLES: usize = 1 << 18;
+/// The most patches a Coons or tensor-product mesh keeps; each is sampled
+/// as up to 722 triangles when drawn.
+pub(crate) const MAX_MESH_PATCHES: usize = 1 << 14;
+
 /// The function supplied to a shading.
 #[derive(Debug, Clone)]
 pub enum ShadingFunction {
@@ -227,7 +236,9 @@
                     .get::<Array<'_>>(DECODE)?
                     .iter::<f32>()
                     .collect::<Vec<_>>();
-                let vertices_per_row = dict.get::<u32>(VERTICES_PER_ROW)?;
+                // Tandem patch: fewer than two vertices per row are read as
+                // two (MuPDF); zero read rows without end.
+                let vertices_per_row = dict.get::<u32>(VERTICES_PER_ROW).unwrap_or(0).max(2);
 
                 let triangles = read_lattice_triangles(
                     stream_data.as_ref(),
@@ -450,9 +461,19 @@
 
     /// Approximate the patch by triangles.
     pub fn to_triangles(&self, buffer: &mut Vec<Triangle>) {
-        generate_patch_triangles(|p| self.map_coordinate(p), |p| self.interpolate(p), buffer);
+        self.to_triangles_in(buffer, GRID_SIZE);
     }
 
+    /// Tandem patch: [`Self::to_triangles`] on a `grid` × `grid` grid.
+    pub(crate) fn to_triangles_in(&self, buffer: &mut Vec<Triangle>, grid: usize) {
+        generate_patch_triangles(
+            |p| self.map_coordinate(p),
+            |p| self.interpolate(p),
+            buffer,
+            grid,
+        );
+    }
+
     /// Get the interpolated colors of the point from the patch.
     pub fn interpolate(&self, pos: Point) -> ColorComponents {
         let (u, v) = (pos.x, pos.y);
@@ -534,9 +555,19 @@
 
     /// Approximate the tensor product patch mesh by triangles.
     pub fn to_triangles(&self, buffer: &mut Vec<Triangle>) {
-        generate_patch_triangles(|p| self.map_coordinate(p), |p| self.interpolate(p), buffer);
+        self.to_triangles_in(buffer, GRID_SIZE);
     }
 
+    /// Tandem patch: [`Self::to_triangles`] on a `grid` × `grid` grid.
+    pub(crate) fn to_triangles_in(&self, buffer: &mut Vec<Triangle>, grid: usize) {
+        generate_patch_triangles(
+            |p| self.map_coordinate(p),
+            |p| self.interpolate(p),
+            buffer,
+            grid,
+        );
+    }
+
     /// Get the interpolated colors of the point from the patch.
     pub fn interpolate(&self, pos: Point) -> ColorComponents {
         let (u, v) = (pos.x, pos.y);
@@ -612,6 +643,9 @@
         }
 
         triangles.push(Triangle::new(a.clone()?, b.clone()?, c.clone()?));
+        if triangles.len() >= MAX_MESH_TRIANGLES {
+            break;
+        }
     }
 
     Some(triangles)
@@ -711,19 +745,27 @@
 }
 
 /// Generate triangles from a grid of points using a mapping function.
-fn generate_patch_triangles<F, I>(map_coordinate: F, interpolate: I, buffer: &mut Vec<Triangle>)
-where
+/// The points a side of the grid a patch is triangulated on.
+pub(crate) const GRID_SIZE: usize = 20;
+
+fn generate_patch_triangles<F, I>(
+    map_coordinate: F,
+    interpolate: I,
+    buffer: &mut Vec<Triangle>,
+    // Tandem patch: the grid's points a side, 2 to GRID_SIZE.
+    grid_size: usize,
+) where
     F: Fn(Point) -> Point,
     I: Fn(Point) -> ColorComponents,
 {
-    const GRID_SIZE: usize = 20;
-    let mut grid = vec![vec![Point::ZERO; GRID_SIZE]; GRID_SIZE];
+    let size = grid_size.clamp(2, GRID_SIZE);
+    let mut grid = vec![vec![Point::ZERO; size]; size];
 
     // Create grid by mapping unit square coordinates.
-    for i in 0..GRID_SIZE {
-        for j in 0..GRID_SIZE {
-            let u = i as f64 / (GRID_SIZE - 1) as f64; // 0.0 to 1.0 (left to right).
-            let v = j as f64 / (GRID_SIZE - 1) as f64; // 0.0 to 1.0 (top to bottom).
+    for i in 0..size {
+        for j in 0..size {
+            let u = i as f64 / (size - 1) as f64; // 0.0 to 1.0 (left to right).
+            let v = j as f64 / (size - 1) as f64; // 0.0 to 1.0 (top to bottom).
 
             // Map unit square coordinate to patch coordinate.
             let unit_point = Point::new(u, v);
@@ -731,18 +773,18 @@
         }
     }
 
-    for i in 0..(GRID_SIZE - 1) {
-        for j in 0..(GRID_SIZE - 1) {
+    for i in 0..(size - 1) {
+        for j in 0..(size - 1) {
             let p00 = grid[i][j];
             let p10 = grid[i + 1][j];
             let p01 = grid[i][j + 1];
             let p11 = grid[i + 1][j + 1];
 
             // Calculate unit square coordinates for color interpolation.
-            let u0 = i as f64 / (GRID_SIZE - 1) as f64;
-            let u1 = (i + 1) as f64 / (GRID_SIZE - 1) as f64;
-            let v0 = j as f64 / (GRID_SIZE - 1) as f64;
-            let v1 = (j + 1) as f64 / (GRID_SIZE - 1) as f64;
+            let u0 = i as f64 / (size - 1) as f64;
+            let u1 = (i + 1) as f64 / (size - 1) as f64;
+            let v0 = j as f64 / (size - 1) as f64;
+            let v1 = (j + 1) as f64 / (size - 1) as f64;
 
             // Create triangle vertices with interpolated colors.
             let v00 = TriangleVertex {
@@ -822,6 +864,9 @@
         })
     };
 
+    // Tandem patch: at most MAX_MESH_TRIANGLES / 2 vertices, which make at
+    // most MAX_MESH_TRIANGLES triangles.
+    let mut vertices = 0;
     'outer: loop {
         let mut single_row = vec![];
 
@@ -831,6 +876,10 @@
             };
 
             single_row.push(next);
+            vertices += 1;
+            if vertices > MAX_MESH_TRIANGLES / 2 {
+                break 'outer;
+            }
         }
 
         lattices.push(single_row);
@@ -971,6 +1020,10 @@
         }
 
         patches.push(create_patch(fixed_points, colors));
+        // Tandem patch: within MAX_MESH_PATCHES.
+        if patches.len() >= MAX_MESH_PATCHES {
+            break;
+        }
     }
     Some(patches)
 }
diff -ru a/src/x_object/decode/image.rs b/src/x_object/decode/image.rs
--- a/src/x_object/decode/image.rs
+++ b/src/x_object/decode/image.rs
@@ -1,13 +1,18 @@
 use super::mask::decode_mask;
-use super::{DecodeContext, decode_context, decode_u8_samples, fix_image_length, unpack_samples};
+use super::{
+    DecodeContext, REDUCED_CACHE_BYTES, box_reduce, decode_context, decode_u8_samples,
+    fix_image_length, reduction, unpack_samples,
+};
 use crate::color::{ColorComponents, ToLuma, ToRgb};
 use crate::interpret::state::ActiveTransferFunction;
+use crate::util::hash128;
 use crate::x_object::image::ImageXObject;
 use crate::{ImageData, LumaData, RgbData};
-use hayro_syntax::object::Stream;
 use hayro_syntax::object::dict::keys::*;
+use hayro_syntax::object::{Object, Stream};
 use smallvec::SmallVec;
 
+#[derive(Clone)]
 pub(crate) struct DecodedImage {
     pub(crate) image: ImageData,
     pub(crate) alpha: Option<LumaData>,
@@ -17,7 +22,91 @@
     obj: &ImageXObject<'_>,
     target_dimension: Option<(u32, u32)>,
 ) -> Option<DecodedImage> {
-    ImageDecoder::new(obj, target_dimension)?.decode()
+    // Tandem patch: an image drawn much smaller than its samples is reduced
+    // by whole-number box factors, and a decoded image is kept for the
+    // render (a page may draw one image many times), unless its decoding
+    // depends on the drawing state (a transfer function, a color space named
+    // in the resources).
+    let (kx, ky) = reduction(obj.width, obj.height, target_dimension);
+    let decode = || {
+        let mut decoded = ImageDecoder::new(obj, target_dimension)?.decode()?;
+        if kx > 1 || ky > 1 {
+            decoded.image = reduce_image(decoded.image, kx, ky);
+        }
+        Some(decoded)
+    };
+    let dict = obj.stream.dict();
+    let named_space = dict
+        .get::<Object<'_>>(CS)
+        .or_else(|| dict.get::<Object<'_>>(COLORSPACE))
+        .is_some_and(|cs| match cs {
+            Object::Name(name) => !matches!(
+                name.as_ref(),
+                DEVICE_RGB | RGB | DEVICE_GRAY | G | DEVICE_CMYK | CMYK | CALCMYK
+            ),
+            _ => false,
+        });
+    if obj.transfer_function.is_some() || named_space {
+        return decode();
+    }
+    let key = hash128(&(
+        b"image",
+        kx,
+        ky,
+        // JPX decoding picks a resolution from the target.
+        target_dimension.map(|(w, h)| (w.next_power_of_two(), h.next_power_of_two())),
+        dict.data(),
+        obj.stream.raw_data().as_ref(),
+    ));
+    let size = |decoded: &DecodedImage| {
+        let image = match &decoded.image {
+            ImageData::Rgb(rgb) => rgb.data.len(),
+            ImageData::Luma(luma) => luma.data.len(),
+        };
+        image + decoded.alpha.as_ref().map_or(0, |a| a.data.len())
+    };
+    obj.cache
+        .get_or_insert_sized(key, REDUCED_CACHE_BYTES, size, decode)
+}
+
+/// Tandem patch: `image` averaged over `kx` × `ky` blocks.
+fn reduce_image(image: ImageData, kx: usize, ky: usize) -> ImageData {
+    let reduce = |data: &[u8], width: u32, height: u32, n: usize| {
+        let (w, h) = (width as usize, height as usize);
+        (
+            box_reduce(data, w, h, n, kx, ky),
+            w.div_ceil(kx) as u32,
+            h.div_ceil(ky) as u32,
+        )
+    };
+    let scale = |(sx, sy): (f32, f32), width: u32, height: u32, w: u32, h: u32| {
+        (
+            sx * width as f32 / w.max(1) as f32,
+            sy * height as f32 / h.max(1) as f32,
+        )
+    };
+    match image {
+        ImageData::Rgb(rgb) => {
+            let (data, width, height) = reduce(&rgb.data, rgb.width, rgb.height, 3);
+            ImageData::Rgb(RgbData {
+                data,
+                scale_factors: scale(rgb.scale_factors, rgb.width, rgb.height, width, height),
+                width,
+                height,
+                interpolate: rgb.interpolate,
+            })
+        }
+        ImageData::Luma(luma) => {
+            let (data, width, height) = reduce(&luma.data, luma.width, luma.height, 1);
+            ImageData::Luma(LumaData {
+                data,
+                scale_factors: scale(luma.scale_factors, luma.width, luma.height, width, height),
+                width,
+                height,
+                interpolate: luma.interpolate,
+            })
+        }
+    }
 }
 
 struct ImageDecoder<'a, 'b> {
diff -ru a/src/x_object/decode/mask.rs b/src/x_object/decode/mask.rs
--- a/src/x_object/decode/mask.rs
+++ b/src/x_object/decode/mask.rs
@@ -1,6 +1,10 @@
-use super::{DecodeContext, decode_context, fix_image_length, unpack_samples};
+use super::{
+    DecodeContext, REDUCED_CACHE_BYTES, box_reduce, decode_context, fix_image_length, reduction,
+    unpack_samples,
+};
 use crate::LumaData;
 use crate::function::interpolate;
+use crate::util::hash128;
 use crate::x_object::image::{ImageKind, ImageXObject};
 use std::sync::LazyLock;
 
@@ -24,6 +28,40 @@
     obj: &ImageXObject<'_>,
     target_dimension: Option<(u32, u32)>,
 ) -> Option<DecodedMask> {
+    // Tandem patch: a mask drawn much smaller than its samples is decoded
+    // reduced by whole-number box factors, once per render.
+    let (kx, ky) = reduction(obj.width, obj.height, target_dimension);
+    if kx > 1 || ky > 1 {
+        let invert = obj.kind == ImageKind::StencilMask;
+        let key = hash128(&(
+            b"mask",
+            kx,
+            ky,
+            invert,
+            obj.stream.dict().data(),
+            obj.stream.raw_data().as_ref(),
+        ));
+        let luma = obj.cache.get_or_insert_sized(
+            key,
+            REDUCED_CACHE_BYTES,
+            |luma: &LumaData| luma.data.len(),
+            || {
+                let ctx = decode_context(obj, target_dimension)?;
+                // A decoder that already reduced the samples leaves less.
+                let (kx, ky) = reduction(ctx.width, ctx.height, target_dimension);
+                let scale_factors = ctx.scale_factors;
+                let (data, width, height) = decode_mask_data_reduced(ctx, invert, kx, ky)?;
+                Some(LumaData {
+                    data,
+                    width: width as u32,
+                    height: height as u32,
+                    interpolate: obj.interpolate,
+                    scale_factors: (scale_factors.0 * kx as f32, scale_factors.1 * ky as f32),
+                })
+            },
+        )?;
+        return Some(DecodedMask { luma });
+    }
     let ctx = decode_context(obj, target_dimension)?;
     let width = ctx.width;
     let scale_factors = ctx.scale_factors;
@@ -115,6 +153,88 @@
     Some((data, ctx.height))
 }
 
+/// Tandem patch: `decode_mask_data` reduced by `kx` × `ky` box factors; a
+/// one-bit mask is counted from its bits without expanding them.
+fn decode_mask_data_reduced(
+    ctx: DecodeContext<'_>,
+    invert: bool,
+    kx: usize,
+    ky: usize,
+) -> Option<(Vec<u8>, usize, usize)> {
+    let width = ctx.width as usize;
+    let inverted_default = ctx
+        .color_space
+        .inverted_default_decode_arr(ctx.bits_per_component as f32);
+    let default_decode = ctx
+        .color_space
+        .default_decode_arr(ctx.bits_per_component as f32);
+    let bilevel = ctx.bits_per_component == 1
+        && ctx.color_space.num_components() == 1
+        && (ctx.decode_arr.as_slice() == default_decode.as_slice()
+            || ctx.decode_arr.as_slice() == inverted_default.as_slice());
+    if !bilevel {
+        let (data, height) = decode_mask_data(ctx, invert)?;
+        let height = height as usize;
+        let reduced = box_reduce(&data, width, height, 1, kx, ky);
+        return Some((reduced, width.div_ceil(kx), height.div_ceil(ky)));
+    }
+    let invert = invert ^ (ctx.decode_arr.as_slice() == inverted_default.as_slice());
+    let height = ctx.height as usize;
+    let out_width = width.div_ceil(kx);
+    let mut counts = vec![0usize; out_width];
+    let mut out = Vec::with_capacity(out_width * height.div_ceil(ky));
+    let mut rows = ctx.decoded.data.chunks(width.div_ceil(8)).take(height);
+    loop {
+        counts.iter_mut().for_each(|c| *c = 0);
+        let mut row_count = 0;
+        for row in rows.by_ref().take(ky) {
+            row_count += 1;
+            count_ones(row, width, kx, &mut counts);
+        }
+        if row_count == 0 {
+            break;
+        }
+        for (bx, count) in counts.iter().enumerate() {
+            let n = (((bx + 1) * kx).min(width) - bx * kx) * row_count;
+            let set = if invert { n - count } else { *count };
+            out.push(((set * 255 + n / 2) / n) as u8);
+        }
+    }
+    let mut out_height = height.div_ceil(ky) as u32;
+    fix_image_length(&mut out, out_width as u32, &mut out_height, 0, 1)?;
+    Some((out, out_width, out_height as usize))
+}
+
+/// Adds the set bits of the first `width` bits of `row` (most significant
+/// first) to `counts`, one count per `kx` bits.
+fn count_ones(row: &[u8], width: usize, kx: usize, counts: &mut [usize]) {
+    for (index, &byte) in row.iter().enumerate() {
+        let start = index * 8;
+        if start >= width {
+            break;
+        }
+        let end = (start + 8).min(width);
+        let byte = byte & !(0xFFu16 >> (end - start)) as u8;
+        if byte == 0 {
+            continue;
+        }
+        let (first, last) = (start / kx, (end - 1) / kx);
+        if first == last {
+            if let Some(count) = counts.get_mut(first) {
+                *count += byte.count_ones() as usize;
+            }
+        } else {
+            for bit in 0..end - start {
+                if byte & (0x80 >> bit) != 0
+                    && let Some(count) = counts.get_mut((start + bit) / kx)
+                {
+                    *count += 1;
+                }
+            }
+        }
+    }
+}
+
 fn decode_bilevel_mask_data(ctx: &DecodeContext<'_>, should_invert: bool) -> Vec<u8> {
     let width = ctx.width as usize;
     let height = ctx.height as usize;
@@ -125,7 +245,12 @@
     let full_bytes = width / 8;
     let tail_bits = width % 8;
     let lut = &**BILEVEL_MASK_LUT;
-    let mut out = Vec::with_capacity(width * height);
+    // Tandem patch: reserved for the rows the data holds, not the declared size.
+    let mut out = Vec::with_capacity(
+        width
+            .saturating_mul(height)
+            .min(data.len().saturating_mul(8)),
+    );
     // To avoid repeatedly calling `extend`, which turns out to be very
     // expensive.
     let mut buffer = [[0; 8]; 32];
diff -ru a/src/x_object/decode/mod.rs b/src/x_object/decode/mod.rs
--- a/src/x_object/decode/mod.rs
+++ b/src/x_object/decode/mod.rs
@@ -111,6 +111,86 @@
     })
 }
 
+/// Tandem patch: decoded images a render keeps, so an image drawn many
+/// times is decoded once.
+pub(crate) const REDUCED_CACHE_BYTES: usize = 32 << 20;
+
+/// Tandem patch: whole-number box factors for an image of `width` ×
+/// `height` samples drawn at `target` pixels, along the axes where it is
+/// drawn more than BOX_REDUCTION times smaller; the renderer's resampler does
+/// the rest.
+pub(crate) fn reduction(width: u32, height: u32, target: Option<(u32, u32)>) -> (usize, usize) {
+    const BOX_REDUCTION: u32 = 8;
+    let Some((target_width, target_height)) = target else {
+        return (1, 1);
+    };
+    let factor = |from: u32, to: u32| {
+        let ratio = from / to.max(1);
+        if ratio > BOX_REDUCTION {
+            (ratio / (BOX_REDUCTION / 2)) as usize
+        } else {
+            1
+        }
+    };
+    (factor(width, target_width), factor(height, target_height))
+}
+
+/// Tandem patch: `data` (`width` × `height` pixels of `n` bytes) averaged
+/// over `kx` × `ky` blocks; partial blocks at the edges average what they
+/// hold.
+pub(crate) fn box_reduce(
+    data: &[u8],
+    width: usize,
+    height: usize,
+    n: usize,
+    kx: usize,
+    ky: usize,
+) -> Vec<u8> {
+    let (out_width, out_height) = (width.div_ceil(kx), height.div_ceil(ky));
+    let mut out = Vec::with_capacity(out_width * out_height * n);
+    if width == 0 || n == 0 {
+        return out;
+    }
+    let mut sums = vec![0u64; out_width * n];
+    for block_rows in data.chunks(width * n * ky).take(out_height) {
+        sums.iter_mut().for_each(|s| *s = 0);
+        let row_count = block_rows.len().div_ceil(width * n);
+        for row in block_rows.chunks(width * n) {
+            for (bx, block) in row.chunks(kx * n).enumerate() {
+                let sum = &mut sums[bx * n..(bx + 1) * n];
+                for pixel in block.chunks_exact(n) {
+                    for (s, v) in sum.iter_mut().zip(pixel) {
+                        *s += u64::from(*v);
+                    }
+                }
+            }
+        }
+        for bx in 0..out_width {
+            let count = ((((bx + 1) * kx).min(width) - bx * kx) * row_count) as u64;
+            for s in &sums[bx * n..(bx + 1) * n] {
+                out.push(((s + count / 2) / count) as u8);
+            }
+        }
+    }
+    out.resize(out_width * out_height * n, 0);
+    out
+}
+
+/// Tandem patch: the most samples an image is unpacked into sample by sample
+/// (1 to 7 and 9 to 15 bits per component): its declared size, missing
+/// data padded, so a few bytes could ask for gigabytes.
+pub(crate) const MAX_UNPACKED_SAMPLES: u64 = 1 << 28;
+
+/// Tandem patch: `width` × `height` × `n` within MAX_UNPACKED_SAMPLES.
+fn unpacked_samples(width: u32, height: u32, n: usize) -> Option<usize> {
+    let samples = u64::from(width) * u64::from(height) * n as u64;
+    if samples > MAX_UNPACKED_SAMPLES {
+        warn!("image of {width} x {height} x {n} samples exceeds the unpacking budget");
+        return None;
+    }
+    usize::try_from(samples).ok()
+}
+
 #[must_use]
 fn fix_image_length<T: Copy>(
     image: &mut Vec<T>,
@@ -150,7 +230,7 @@
 ) -> Option<Vec<u8>> {
     let source_max = 2.0_f32.powi(bits_per_component as i32) - 1.0;
     let num_components = color_space.num_components() as usize;
-    let capacity = width as usize * height as usize * num_components;
+    let capacity = unpacked_samples(width, height, num_components)?;
     let ranges = color_space.component_ranges();
     let indexed_hival = color_space.indexed_hival();
 
@@ -217,7 +297,7 @@
     num_components: usize,
     bits_per_component: u8,
 ) -> Option<Vec<u16>> {
-    let capacity = width as usize * height as usize * num_components;
+    let capacity = unpacked_samples(width, height, num_components)?;
 
     match bits_per_component {
         1..8 | 9..16 => {
```
