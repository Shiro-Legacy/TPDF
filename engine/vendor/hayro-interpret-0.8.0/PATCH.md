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
  - An image's samples are unpacked within `InterpreterSettings::max_image_samples`
    (`MAX_IMAGE_SAMPLES`, 2^28, in `src/interpret/mod.rs`): an image of 1 to 7
    or 9 to 15 bits per component into its declared height (missing data read
    as 0, as MuPDF pads it) while that fits, otherwise, and at 8 or 16 bits,
    into the rows its data holds (`unpacked_rows`). A few bytes declaring
    100,000 × 100,000 samples asked for 10 GB.
  - Samples past the budget are decoded reduced toward the size drawn, a
    strip of rows at a time, with a colour key mask
    (`decode_image_reduced`); a one-bit mask is counted from its bits. An
    image not drawn smaller, or a mask of more than one bit, past the budget
    is not decoded and `InterpreterWarning::BudgetExceeded` is sent (the
    engine then fails the render instead of drawing the page without it).
  - Sizes are u64 or checked: on wasm32 `fix_image_length`'s
    `row × height` wrapped, cutting the data to nothing while the height
    stayed, and the box filter then divided by zero (a 716-byte file trapped
    the WASM engine). Every image handed to the renderer holds width ×
    height × components bytes.
  - An image or mask drawn more than 8× smaller than its samples is reduced
    by whole-number box factors when decoded (`reduction`, `box_reduce`),
    computed from the decoded image's own size, so a JPX image whose decoder
    skipped resolution levels is not reduced again; a one-bit mask is counted
    from its bits without expanding them (`decode_mask_data_reduced`).
  - A decoded image or reduced mask is kept for the rest of the render
    (`Cache::get_or_insert_sized`, within `REDUCED_CACHE_BYTES`, 32 MiB), so
    an image drawn many times is decoded once (pdf.js `issue16263.pdf` draws
    one 34,862 × 4,332 mask many times), unless its decoding depends on the
    drawing state (a transfer function, a colour space named in the
    resources: `decodes_alike`).
- `src/x_object/decode/budget.rs` (new): the decoders that allocate from
  declared sizes before decoding are held to the image's budget first: DCT
  (zune-jpeg's output at the frame's size, patched down to the dictionary's
  as hayro-syntax does, plus 16-bit coefficients when progressive; its output
  size overflowed `usize` on wasm32 and unwrapped `None`), CCITT (`Columns ×
  max(Rows, Height)` bytes, reserved by hayro-syntax), JBIG2 (the page from
  its page information and stripes, reserved by hayro-syntax, the largest
  region bitmap, and the referred-to segment numbers hayro-jbig2 reserves),
  JPX (4 bytes a sample at the resolution decoded; a JPX image skips more
  resolution levels to fit). JBIG2 or JPX data behind another filter cannot
  be read before decoding and is refused.
- `src/types.rs`, `src/x_object/image.rs`: `RasterImage::shared_key` and
  `StencilImage::shared_key`, a key for what an image decodes to (its
  dictionary and data) when that does not depend on the drawing state, so
  the renderer can share one pixmap between draws.
- `src/interpret/mod.rs`: `InterpreterSettings::max_image_samples`,
  `MAX_IMAGE_SAMPLES` and `InterpreterWarning::BudgetExceeded`, passed to
  every image (`x_object/mod.rs`, `x_object/image.rs`).
- `src/font/cid.rs`: a CID font's /W and /W2 set CIDs up to 0xFFFF only
  (CIDs are two bytes; MuPDF keeps these ranges as 16-bit pairs) and at most
  2^18 codes per array, the rest ignored. hayro set every code of a range,
  so `[0 4294967295 500]` asked for 2^32 entries (an out-of-memory trap) and
  the 200 ranges above 0xFFFF of p3-edit's "/W expansion" test cost about a
  second in every view op. Test: `engine/tests/app_accept_edit.rs`
  `w_ranges_past_two_bytes_cost_the_view_nothing`; notes:
  `engine/design/app-accept-edit.md`.

### Integration acceptance review (October 16, 2026)

Small hostile inputs that trapped the WASM engine or grew its memory by
gigabytes (`engine/design/app-accept-integration.md`; tests
`engine/tests/view_hostile.rs`, `tests/rust-engine-bounds.test.mjs`):

- `src/function/type4.rs`: procedures nest at most `MAX_PROCEDURE_DEPTH`
  (100, MuPDF's "too much nesting in calculator function") and a program has
  at most `MAX_PROGRAM_OPS` (2^14) operators; past either the function is
  not read. Parsing, evaluation and drop recursed without a limit (about
  4,000 levels, 8 KB of program, overflowed the WASM stack).
- `src/color/mod.rs`, `indexed.rs`, `separation.rs`, `device_n.rs`: colour
  spaces nest at most `MAX_COLOR_SPACE_DEPTH` (8) deep (Indexed base,
  Separation and DeviceN alternate, ICCBased alternate, Pattern space); past
  it the space is not read. About 2,000 overflowed the WASM stack.
- `src/function/mod.rs`, `type3.rs`: `Function::new_with` builds a function
  with the render's cache: stitching functions nest at most
  `MAX_FUNCTION_DEPTH` (32; MuPDF draws 128 and not 256), one build reads at
  most `MAX_FUNCTION_NODES` (4,096) functions, and a function is built once
  per cache, keyed by where its dictionary is (a part shared, or referring
  back, took exponential time; a chain of 2,000 overflowed the WASM stack).
  A stitching function's bounds are collected once, at most
  `MAX_STITCHING_BOUNDS` (1,024): they were collected anew for every path
  through shared parts (30 levels each naming the next twice: 2^30 walks).
  Shadings, transfer functions, soft masks and tint transforms pass the
  cache (`shading.rs`, `interpret/state.rs`, `x_object/soft_mask.rs`).
- `src/function/type0.rs`, `src/cache.rs`, `src/context.rs`: a sampled
  function's table holds its declared entries, at most what its data holds
  (hayro kept a 4-byte entry for every bit of data at one bit a sample, past
  the declared size), and the tables built with one cache take at most
  `MAX_SAMPLED_BYTES` (64 MiB); past it the function is not built and
  `InterpreterCache::over_budget` reports it.
- `src/x_object/decode/mod.rs`: image data past hayro-syntax's decoding
  limit (`DecodeFailure::TooLarge`) is over budget
  (`InterpreterWarning::BudgetExceeded`), not a decoding failure.
- `src/interpret/mod.rs`: `interpret_page` reads the page's content with
  `Page::page_stream` (patched hayro-syntax keeps it only within a budget
  for the document) and releases it after the content runs.

## Diff against the published crate

```diff
diff -ru a/src/cache.rs b/src/cache.rs
--- a/src/cache.rs
+++ b/src/cache.rs
@@ -50,6 +50,89 @@
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
+// Tandem patch: the bytes of sampled function tables built with one cache.
+/// The key holding the bytes reserved by `reserve_sampled`.
+const SAMPLED_TOTAL: u128 = u128::MAX - 1;
+/// The key marking a reservation refused.
+const OVER_BUDGET: u128 = u128::MAX - 2;
+/// Bytes of sampled (type 0) function tables one cache (a render, or the
+/// text of a document) may build.
+pub(crate) const MAX_SAMPLED_BYTES: usize = 64 << 20;
+
+impl Cache {
+    /// Counts `bytes` of sampled function tables against
+    /// [`MAX_SAMPLED_BYTES`]: false, and the cache over budget, past it.
+    pub(crate) fn reserve_sampled(&self, bytes: usize) -> bool {
+        let mut locked = self
+            .0
+            .lock()
+            .unwrap_or_else(std::sync::PoisonError::into_inner);
+        let fits = locked
+            .entry(SAMPLED_TOTAL)
+            .or_insert_with(|| Some(Box::new(0usize)))
+            .as_mut()
+            .and_then(|total| total.downcast_mut::<usize>())
+            .filter(|total| total.saturating_add(bytes) <= MAX_SAMPLED_BYTES)
+            .map(|total| *total += bytes)
+            .is_some();
+        if !fits {
+            locked.insert(OVER_BUDGET, None);
+        }
+        fits
+    }
+
+    /// Whether something built with this cache passed its budget.
+    pub(crate) fn over_budget(&self) -> bool {
+        self.0
+            .lock()
+            .unwrap_or_else(std::sync::PoisonError::into_inner)
+            .contains_key(&OVER_BUDGET)
+    }
+}
+
 /// A trait for objects that can generate a unique cache key.
 pub trait CacheKey {
     /// Returns the cache key for this object.
diff -ru a/src/color/device_n.rs b/src/color/device_n.rs
--- a/src/color/device_n.rs
+++ b/src/color/device_n.rs
@@ -13,7 +13,7 @@
 }
 
 impl DeviceN {
-    pub(super) fn new(array: &Array<'_>, cache: &Cache) -> Option<Self> {
+    pub(super) fn new(array: &Array<'_>, cache: &Cache, depth: u8) -> Option<Self> {
         let mut iter = array.flex_iter();
         // Skip `/DeviceN`
         let _ = iter.next::<Name<'_>>()?;
@@ -24,8 +24,8 @@
             .collect::<Vec<_>>();
         let num_components = u8::try_from(names.len()).ok()?;
         let all_none = names.iter().all(|n| n.as_str() == "None");
-        let alternate_space = ColorSpace::new(iter.next::<Object<'_>>()?, cache)?;
-        let tint_transform = Function::new(&iter.next::<Object<'_>>()?)?;
+        let alternate_space = ColorSpace::nested(iter.next::<Object<'_>>()?, cache, depth)?;
+        let tint_transform = Function::new_with(&iter.next::<Object<'_>>()?, cache)?;
 
         if num_components == 0 {
             return None;
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
diff -ru a/src/color/indexed.rs b/src/color/indexed.rs
--- a/src/color/indexed.rs
+++ b/src/color/indexed.rs
@@ -13,11 +13,11 @@
 }
 
 impl Indexed {
-    pub(super) fn new(array: &Array<'_>, cache: &Cache) -> Option<Self> {
+    pub(super) fn new(array: &Array<'_>, cache: &Cache, depth: u8) -> Option<Self> {
         let mut iter = array.flex_iter();
         // Skip name
         let _ = iter.next::<Name<'_>>()?;
-        let base_color_space = ColorSpace::new(iter.next::<Object<'_>>()?, cache)?;
+        let base_color_space = ColorSpace::nested(iter.next::<Object<'_>>()?, cache, depth)?;
         let hival = iter.next::<u32>()?.min(u8::MAX as u32) as u8;
 
         let values = {
diff -ru a/src/color/mod.rs b/src/color/mod.rs
--- a/src/color/mod.rs
+++ b/src/color/mod.rs
@@ -156,12 +156,25 @@
     DeviceN(DeviceN),
 }
 
+/// Tandem patch: how deep colour spaces may nest (an Indexed base, a
+/// Separation or DeviceN alternate, an ICCBased alternate, a Pattern's
+/// underlying space). hayro recursed without a limit: a chain of about 2,000
+/// such spaces overflowed the WASM stack. Valid files nest at most about 4.
+pub(crate) const MAX_COLOR_SPACE_DEPTH: u8 = 8;
+
 impl ColorSpaceType {
-    fn new(object: Object<'_>, cache: &Cache) -> Option<Self> {
-        Self::new_inner(object, cache)
+    fn new(object: Object<'_>, cache: &Cache, depth: u8) -> Option<Self> {
+        // Tandem patch: past the depth the space is not read.
+        if depth > MAX_COLOR_SPACE_DEPTH {
+            warn!("color spaces nest too deeply");
+
+            return None;
+        }
+
+        Self::new_inner(object, cache, depth)
     }
 
-    fn new_inner(object: Object<'_>, cache: &Cache) -> Option<Self> {
+    fn new_inner(object: Object<'_>, cache: &Cache, depth: u8) -> Option<Self> {
         if let Object::Name(name) = object {
             return Self::new_from_name(&name);
         } else if let Object::Array(color_array) = object {
@@ -190,7 +203,7 @@
                                 })
                                 .or_else(|| {
                                     dict.get::<Object<'_>>(ALTERNATE)
-                                        .and_then(|o| Self::new(o, cache))
+                                        .and_then(|o| Self::new(o, cache, depth + 1))
                                 })
                                 .or_else(|| match dict.get::<u8>(N) {
                                     Some(1) => Some(Self::DeviceGray(DeviceGray)),
@@ -220,19 +233,23 @@
                     return Some(Self::Lab(Lab::new(&lab_dict)?));
                 }
                 INDEXED | I => {
-                    return Some(Self::Indexed(Indexed::new(&color_array, cache)?));
+                    return Some(Self::Indexed(Indexed::new(&color_array, cache, depth)?));
                 }
                 SEPARATION => {
-                    return Some(Self::Separation(Separation::new(&color_array, cache)?));
+                    return Some(Self::Separation(Separation::new(
+                        &color_array,
+                        cache,
+                        depth,
+                    )?));
                 }
                 DEVICE_N => {
-                    return Some(Self::DeviceN(DeviceN::new(&color_array, cache)?));
+                    return Some(Self::DeviceN(DeviceN::new(&color_array, cache, depth)?));
                 }
                 PATTERN => {
                     let _ = iter.next::<Name<'_>>();
                     let cs = iter
                         .next::<Object<'_>>()
-                        .and_then(|o| ColorSpace::new(o, cache))
+                        .and_then(|o| ColorSpace::nested(o, cache, depth))
                         .unwrap_or(ColorSpace::device_rgb());
                     return Some(Self::Pattern(Pattern::new(cs)));
                 }
@@ -265,7 +282,16 @@
 impl ColorSpace {
     /// Create a new color space from the given object.
     pub(crate) fn new(object: Object<'_>, cache: &Cache) -> Option<Self> {
-        Some(Self(Arc::new(ColorSpaceType::new(object, cache)?)))
+        Some(Self(Arc::new(ColorSpaceType::new(object, cache, 0)?)))
+    }
+
+    /// Tandem patch: a colour space within one at `depth`.
+    pub(super) fn nested(object: Object<'_>, cache: &Cache, depth: u8) -> Option<Self> {
+        Some(Self(Arc::new(ColorSpaceType::new(
+            object,
+            cache,
+            depth + 1,
+        )?)))
     }
 
     /// Create a new color space from the name.
diff -ru a/src/color/separation.rs b/src/color/separation.rs
--- a/src/color/separation.rs
+++ b/src/color/separation.rs
@@ -13,13 +13,13 @@
 }
 
 impl Separation {
-    pub(super) fn new(array: &Array<'_>, cache: &Cache) -> Option<Self> {
+    pub(super) fn new(array: &Array<'_>, cache: &Cache, depth: u8) -> Option<Self> {
         let mut iter = array.flex_iter();
         // Skip `/Separation`
         let _ = iter.next::<Name<'_>>()?;
         let name = iter.next::<Name<'_>>()?;
-        let alternate_space = ColorSpace::new(iter.next::<Object<'_>>()?, cache)?;
-        let tint_transform = Function::new(&iter.next::<Object<'_>>()?)?;
+        let alternate_space = ColorSpace::nested(iter.next::<Object<'_>>()?, cache, depth)?;
+        let tint_transform = Function::new_with(&iter.next::<Object<'_>>()?, cache)?;
         // Either I did something wrong, or no other viewers properly handles
         // `All`, so let's just ignore it as well.
         let is_none_separation = name.as_str() == "None";
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
diff -ru a/src/context.rs b/src/context.rs
--- a/src/context.rs
+++ b/src/context.rs
@@ -37,6 +37,12 @@
 }
 
 impl<'a> InterpreterCache<'a> {
+    /// Tandem patch: whether something built with this cache passed its
+    /// budget (sampled function tables), so a render with it is incomplete.
+    pub fn over_budget(&self) -> bool {
+        self.object_cache.over_budget()
+    }
+
     /// Create a new interpreter cache.
     pub fn new() -> Self {
         Self {
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
diff -ru a/src/font/cid.rs b/src/font/cid.rs
--- a/src/font/cid.rs
+++ b/src/font/cid.rs
@@ -509,18 +509,37 @@
     }
 }
 
+/// Tandem patch: a /W or /W2 array sets CIDs up to 0xFFFF only (CIDs are
+/// two bytes; MuPDF keeps these ranges as 16-bit pairs), and at most
+/// `MAX_WIDTH_CODES` codes in all, the rest of the array ignored. hayro set
+/// every code of a range one by one, so `[0 4294967295 500]` (a few bytes)
+/// asked for 2^32 entries, and 200 ranges above 0xFFFF took a second.
+const MAX_CID: u32 = 0xFFFF;
+const MAX_WIDTH_CODES: usize = 1 << 18;
+
 fn read_widths(arr: &Array<'_>) -> Option<FxHashMap<u32, f32>> {
     let mut map = FxHashMap::default();
     let mut iter = arr.flex_iter();
+    let mut room = MAX_WIDTH_CODES;
 
     loop {
         if let Some((mut first, range)) = iter.next::<(u32, Array<'_>)>() {
             for width in range.iter::<f32>() {
-                map.insert(first, width);
+                if room == 0 {
+                    return Some(map);
+                }
+                room -= 1;
+                if first <= MAX_CID {
+                    map.insert(first, width);
+                }
                 first = first.checked_add(1)?;
             }
         } else if let Some((first, second, width)) = iter.next::<(u32, u32, f32)>() {
-            for i in first..=second {
+            for i in first..=second.min(MAX_CID) {
+                if room == 0 {
+                    return Some(map);
+                }
+                room -= 1;
                 map.insert(i, width);
             }
         } else {
@@ -534,6 +553,7 @@
 fn read_widths2(arr: &Array<'_>) -> Option<FxHashMap<u32, [f32; 3]>> {
     let mut map = FxHashMap::default();
     let mut iter = arr.flex_iter();
+    let mut room = MAX_WIDTH_CODES;
 
     loop {
         if let Some((mut first, range)) = iter.next::<(u32, Array<'_>)>() {
@@ -542,11 +562,21 @@
             while let Some(w) = iter.next() {
                 let v1 = iter.next()?;
                 let v2 = iter.next()?;
-                map.insert(first, [w, v1, v2]);
+                if room == 0 {
+                    return Some(map);
+                }
+                room -= 1;
+                if first <= MAX_CID {
+                    map.insert(first, [w, v1, v2]);
+                }
                 first = first.checked_add(1)?;
             }
         } else if let Some((first, second, w, v1, v2)) = iter.next::<(u32, u32, f32, f32, f32)>() {
-            for i in first..=second {
+            for i in first..=second.min(MAX_CID) {
+                if room == 0 {
+                    return Some(map);
+                }
+                room -= 1;
                 map.insert(i, [w, v1, v2]);
             }
         } else {
diff -ru a/src/function/mod.rs b/src/function/mod.rs
--- a/src/function/mod.rs
+++ b/src/function/mod.rs
@@ -8,6 +8,8 @@
 mod type3;
 mod type4;
 
+use crate::cache::Cache;
+use crate::util::hash128;
 use crate::function::type0::Type0;
 use crate::function::type2::Type2;
 use crate::function::type3::Type3;
@@ -35,20 +37,69 @@
 #[derive(Debug, Clone)]
 pub struct Function(Arc<FunctionType>);
 
+/// Tandem patch: how deep stitching (type 3) functions may nest. hayro
+/// recursed without a limit: a chain of about 2,000 overflowed the WASM
+/// stack. MuPDF 1.28.1 draws 128 levels and not 256.
+pub(crate) const MAX_FUNCTION_DEPTH: u32 = 32;
+/// Tandem patch: functions one function may be built from (a stitching
+/// function's sub-functions, at every level): their objects may be shared or
+/// refer back, which without a bound took exponential time.
+pub(crate) const MAX_FUNCTION_NODES: u32 = 4096;
+
+/// Tandem patch: what building one function has used.
+#[derive(Default)]
+pub(crate) struct Building {
+    depth: u32,
+    nodes: u32,
+}
+
 impl Function {
     /// Create a new function.
     pub fn new(obj: &Object<'_>) -> Option<Self> {
+        Self::new_with(obj, &Cache::new())
+    }
+
+    /// Tandem patch: a function built with `cache`, where functions defined
+    /// by an object are kept (sampled ones count against its budget).
+    pub(crate) fn new_with(obj: &Object<'_>, cache: &Cache) -> Option<Self> {
+        Self::build(obj, cache, &mut Building::default())
+    }
+
+    /// Tandem patch: a function within the bounds of `building`.
+    pub(crate) fn build(obj: &Object<'_>, cache: &Cache, building: &mut Building) -> Option<Self> {
         let (dict, stream) = dict_or_stream(obj)?;
+        if building.depth >= MAX_FUNCTION_DEPTH || building.nodes >= MAX_FUNCTION_NODES {
+            warn!("function nests too deeply or has too many parts");
 
-        let function_type = match dict.get::<u8>(FUNCTION_TYPE)? {
-            0 => FunctionType::Type0(Type0::new(stream?)?),
-            2 => FunctionType::Type2(Type2::new(dict)?),
-            3 => FunctionType::Type3(Type3::new(dict)?),
-            4 => FunctionType::Type4(Type4::new(stream?)?),
-            _ => return None,
-        };
+            return None;
+        }
+        building.nodes += 1;
+        // Kept by where the dictionary is, so a function shared by several
+        // colour spaces, shadings or stitching functions is built once (and
+        // a sampled one's table counted once).
+        let data = dict.data();
+        let key = hash128(&(
+            b"function",
+            dict.obj_id(),
+            data.as_ptr().addr(),
+            data.len(),
+        ));
+        cache.get_or_insert_with(key, || {
+            let function_type = match dict.get::<u8>(FUNCTION_TYPE)? {
+                0 => FunctionType::Type0(Type0::new(stream?, cache)?),
+                2 => FunctionType::Type2(Type2::new(dict)?),
+                3 => {
+                    building.depth += 1;
+                    let function = Type3::new(dict, cache, building);
+                    building.depth -= 1;
+                    FunctionType::Type3(function?)
+                }
+                4 => FunctionType::Type4(Type4::new(stream?)?),
+                _ => return None,
+            };
 
-        Some(Self(Arc::new(function_type)))
+            Some(Self(Arc::new(function_type)))
+        })
     }
 
     /// Evaluate the function with the given input.
diff -ru a/src/function/type0.rs b/src/function/type0.rs
--- a/src/function/type0.rs
+++ b/src/function/type0.rs
@@ -1,3 +1,4 @@
+use crate::cache::Cache;
 use crate::function::{Clamper, TupleVec, Values, interpolate};
 use hayro_syntax::bit_reader::BitReader;
 use hayro_syntax::object::Array;
@@ -20,7 +21,7 @@
 
 impl Type0 {
     /// Create a new type 0 function.
-    pub(crate) fn new(stream: &Stream<'_>) -> Option<Self> {
+    pub(crate) fn new(stream: &Stream<'_>, cache: &Cache) -> Option<Self> {
         let dict = stream.dict();
         let bits_per_sample = dict.get::<u8>(BITS_PER_SAMPLE)?;
 
@@ -50,29 +51,41 @@
 
         let decode = dict.get::<TupleVec>(DECODE).unwrap_or(range.clone());
 
-        let mut data = {
+        let mut stride = range.len();
+        let mut strides = Vec::with_capacity(sizes.len());
+        for size in &sizes {
+            strides.push(stride);
+            stride = stride.checked_mul(*size as usize)?;
+        }
+        let num_expected_entries = stride;
+
+        // Tandem patch: at most the entries the function has, each 4 bytes
+        // within the cache's budget for sampled tables. hayro read every
+        // sample the data held, a 4-byte entry per bit at one bit per
+        // sample: a few kilobytes of nested Flate asked for gigabytes.
+        let data = {
             let decoded = stream.decoded().ok()?;
-            let mut buf = vec![];
+            let held = decoded.len().saturating_mul(8) / usize::from(bits_per_sample);
+            let entries = num_expected_entries.min(held);
+            if !cache.reserve_sampled(entries.saturating_mul(4)) {
+                warn!("sampled function tables exceed their budget");
+
+                return None;
+            }
+            let mut buf = Vec::with_capacity(entries);
             let mut reader = BitReader::new(&decoded);
 
-            while let Some(data) = reader.read(bits_per_sample) {
+            while buf.len() < entries
+                && let Some(data) = reader.read(bits_per_sample)
+            {
                 buf.push(data);
             }
 
             buf
         };
 
-        let mut stride = range.len();
-        let mut strides = Vec::with_capacity(sizes.len());
-        for size in &sizes {
-            strides.push(stride);
-            stride = stride.checked_mul(*size as usize)?;
-        }
-        let num_expected_entries = stride;
-
         if data.len() != num_expected_entries {
             warn!("Type0 function didn't have the expected number of sample entries.");
-            data.truncate(num_expected_entries);
         }
 
         Some(Self {
diff -ru a/src/function/type3.rs b/src/function/type3.rs
--- a/src/function/type3.rs
+++ b/src/function/type3.rs
@@ -1,10 +1,20 @@
-use crate::function::{Clamper, Function, StitchingBounds, TupleVec, Values, interpolate};
+use crate::cache::Cache;
+use crate::function::{
+    Building, Clamper, Function, StitchingBounds, TupleVec, Values, interpolate,
+};
 use hayro_syntax::object::Array;
 use hayro_syntax::object::Dict;
 use hayro_syntax::object::Object;
 use hayro_syntax::object::dict::keys::{BOUNDS, ENCODE, FUNCTIONS};
 use smallvec::smallvec;
+use std::sync::OnceLock;
 
+/// Tandem patch: the stitching bounds one function reports. A function's
+/// bounds include its parts' (each a stitching function maybe shared by
+/// several parents), which hayro collected anew for every path through
+/// them: 30 levels each naming the next twice took 2^30 walks and gigabytes.
+const MAX_STITCHING_BOUNDS: usize = 1024;
+
 /// A type 3 function (stitching function).
 #[derive(Debug)]
 pub(crate) struct Type3 {
@@ -12,16 +22,21 @@
     bounds: Vec<f32>,
     encode: TupleVec,
     clamper: Clamper,
+    // Tandem patch: the stitching bounds, collected once.
+    stitching: OnceLock<StitchingBounds>,
 }
 
 impl Type3 {
     /// Create a new type 3 function.
-    pub(crate) fn new(dict: &Dict<'_>) -> Option<Self> {
+    pub(crate) fn new(dict: &Dict<'_>, cache: &Cache, building: &mut Building) -> Option<Self> {
         let clamper = Clamper::new(dict)?;
 
-        let functions = dict
-            .get::<Array<'_>>(FUNCTIONS)
-            .and_then(|d| d.iter::<Object<'_>>().map(|o| Function::new(&o)).collect())?;
+        // Tandem patch: sub-functions within the bounds of `building`.
+        let functions = dict.get::<Array<'_>>(FUNCTIONS).and_then(|d| {
+            d.iter::<Object<'_>>()
+                .map(|o| Function::build(&o, cache, building))
+                .collect()
+        })?;
         let domain = *clamper.domain.first()?;
         let mut bounds = vec![domain.0 - 0.0001];
         if let Some(a) = dict.get::<Array<'_>>(BOUNDS) {
@@ -37,6 +52,7 @@
             clamper,
             bounds,
             encode,
+            stitching: OnceLock::new(),
         })
     }
 
@@ -62,7 +78,19 @@
         Some(evaluated)
     }
 
+    /// Tandem patch: collected once, at most `MAX_STITCHING_BOUNDS` (the
+    /// lowest).
     pub(crate) fn stitching_bounds(&self) -> StitchingBounds {
+        self.stitching
+            .get_or_init(|| {
+                let mut bounds = self.collect_stitching_bounds();
+                bounds.truncate(MAX_STITCHING_BOUNDS);
+                bounds
+            })
+            .clone()
+    }
+
+    fn collect_stitching_bounds(&self) -> StitchingBounds {
         let mut stitching_bounds = StitchingBounds::new();
         if self.bounds.len() > 2 {
             stitching_bounds.extend_from_slice(&self.bounds[1..self.bounds.len() - 1]);
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
 
@@ -414,12 +426,33 @@
     Some(())
 }
 
+/// Tandem patch: how deep procedures may nest. hayro parsed (and evaluated,
+/// and dropped) them recursively without a limit: about 4,000 levels, 8 KB
+/// of program, overflowed the WASM stack. MuPDF 1.28.1 refuses past 100 ("too
+/// much nesting in calculator function").
+const MAX_PROCEDURE_DEPTH: u32 = 100;
+/// Tandem patch: operators a program may have, at every level (a few bytes
+/// of program each, kept as 32-byte operators and run for every value the
+/// function maps).
+const MAX_PROGRAM_OPS: usize = 1 << 14;
+
 fn parse_procedure(data: &[u8]) -> Option<Vec<PostScriptOp>> {
     let mut r = Reader::new(data);
-    parse_procedure_inner(&mut r)
+    let mut ops = 0;
+    parse_procedure_inner(&mut r, 0, &mut ops)
 }
 
-fn parse_procedure_inner(r: &mut Reader<'_>) -> Option<Vec<PostScriptOp>> {
+fn parse_procedure_inner(
+    r: &mut Reader<'_>,
+    depth: u32,
+    count: &mut usize,
+) -> Option<Vec<PostScriptOp>> {
+    if depth > MAX_PROCEDURE_DEPTH {
+        warn!("postscript procedures nest too deeply");
+
+        return None;
+    }
+
     let mut stack = ParseStack::new();
 
     let mut ops = vec![];
@@ -434,8 +467,14 @@
 
             break;
         } else if r.peek_byte()? == b'{' {
-            stack.push(parse_procedure_inner(r)?);
+            stack.push(parse_procedure_inner(r, depth + 1, count)?);
         } else {
+            *count += 1;
+            if *count > MAX_PROGRAM_OPS {
+                warn!("postscript function has too many operators");
+
+                return None;
+            }
             let op = PostScriptOp::from_reader(r, &mut stack)?;
             ops.push(op);
         }
diff -ru a/src/interpret/mod.rs b/src/interpret/mod.rs
--- a/src/interpret/mod.rs
+++ b/src/interpret/mod.rs
@@ -100,8 +100,17 @@
     /// Note that this feature is currently not fully implemented yet, so some
     /// annotations might be missing.
     pub render_annotations: bool,
+    /// Tandem patch: the most samples one image may decode to (see
+    /// [`MAX_IMAGE_SAMPLES`]); past it an image drawn smaller is decoded
+    /// reduced, else it is not drawn and
+    /// [`InterpreterWarning::BudgetExceeded`] is sent.
+    pub max_image_samples: u64,
 }
 
+/// Tandem patch: the samples one image may decode to by default (256 MiB at
+/// a byte a sample).
+pub const MAX_IMAGE_SAMPLES: u64 = 1 << 28;
+
 impl Default for InterpreterSettings {
     fn default() -> Self {
         Self {
@@ -118,6 +127,7 @@
             cmap_resolver: Arc::new(|_| None),
             warning_sink: Arc::new(|_| {}),
             render_annotations: true,
+            max_image_samples: MAX_IMAGE_SAMPLES,
         }
     }
 }
@@ -135,6 +145,10 @@
     /// An annotation has an `AP` dictionary, but its normal appearance could not
     /// be selected or loaded, so the annotation was not drawn.
     UnresolvedAnnotationAppearance,
+    /// Tandem patch: an image would decode past
+    /// [`InterpreterSettings::max_image_samples`] and was not drawn, so the
+    /// drawing is incomplete.
+    BudgetExceeded,
 }
 
 /// interpret the contents of the page and render them into the device.
@@ -144,7 +158,11 @@
     device: &mut impl Device<'a>,
 ) {
     let resources = page.resources();
-    interpret(page.typed_operations(), resources, context, device);
+    // Tandem patch: hayro-syntax keeps a page's decoded content only within
+    // a budget for the document, so it is read here and released after.
+    if let Some(content) = page.page_stream() {
+        interpret(TypedIter::new(&content), resources, context, device);
+    }
 
     if context.settings.render_annotations
         && let Some(annot_arr) = page.raw().get::<Array<'_>>(ANNOTS)
@@ -673,6 +691,7 @@
                 let cache = context.interpreter_cache.object_cache.clone();
                 let warning_sink = context.settings.warning_sink.clone();
                 let transfer_function = context.get().graphics_state.transfer_function.clone();
+                let max_samples = context.settings.max_image_samples;
                 if let Some(x_object) = resources.get_x_object(x.0).and_then(|s| {
                     XObject::new(
                         &s,
@@ -680,6 +699,7 @@
                         &warning_sink,
                         &cache,
                         transfer_function.clone(),
+                        max_samples,
                     )
                 }) {
                     x_object.draw(resources, context, device);
@@ -689,12 +709,14 @@
                 let warning_sink = context.settings.warning_sink.clone();
                 let transfer_function = context.get().graphics_state.transfer_function.clone();
                 let cache = context.interpreter_cache.object_cache.clone();
+                let max_samples = context.settings.max_image_samples;
                 if let Some(x_object) = ImageXObject::new(
                     i.0,
                     |name| context.get_color_space(resources, name),
                     &warning_sink,
                     &cache,
                     transfer_function,
+                    max_samples,
                 ) {
                     x_object.draw(context, device);
                 }
diff -ru a/src/interpret/state.rs b/src/interpret/state.rs
--- a/src/interpret/state.rs
+++ b/src/interpret/state.rs
@@ -346,11 +346,12 @@
             {
                 Object::Array(array) => {
                     let mut iter = array.iter::<Object<'_>>();
+                    let cache = &context.interpreter_cache.object_cache;
                     let functions = [
-                        TransferFunction::new(Function::new(&iter.next()?)?),
-                        TransferFunction::new(Function::new(&iter.next()?)?),
-                        TransferFunction::new(Function::new(&iter.next()?)?),
-                        TransferFunction::new(Function::new(&iter.next()?)?),
+                        TransferFunction::new(Function::new_with(&iter.next()?, cache)?),
+                        TransferFunction::new(Function::new_with(&iter.next()?, cache)?),
+                        TransferFunction::new(Function::new_with(&iter.next()?, cache)?),
+                        TransferFunction::new(Function::new_with(&iter.next()?, cache)?),
                     ];
 
                     Some(ActiveTransferFunction::Four(functions))
@@ -358,7 +359,7 @@
                 // Only `Identity` and `Default` are valid, which both just reset it.
                 Object::Name(_) => None,
                 o => Some(ActiveTransferFunction::Single(TransferFunction::new(
-                    Function::new(&o)?,
+                    Function::new_with(&o, &context.interpreter_cache.object_cache)?,
                 ))),
             };
 
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
@@ -152,7 +161,7 @@
                     .get::<[f64; 6]>(MATRIX)
                     .map(Affine::new)
                     .unwrap_or_default();
-                let function = read_function(dict, &color_space)?;
+                let function = read_function(dict, &color_space, cache)?;
 
                 ShadingType::FunctionBased {
                     domain,
@@ -162,7 +171,7 @@
             }
             2 | 3 => {
                 let domain = dict.get::<[f32; 2]>(DOMAIN).unwrap_or([0.0, 1.0]);
-                let function = read_function(dict, &color_space)?;
+                let function = read_function(dict, &color_space, cache)?;
                 let extend = dict.get::<[bool; 2]>(EXTEND).unwrap_or([false, false]);
                 let (coords, invalid) = if shading_num == 2 {
                     let read = dict.get::<[f32; 4]>(COORDS)?;
@@ -197,7 +206,7 @@
                 let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                 let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
                 let bpf = dict.get::<u8>(BITS_PER_FLAG)?;
-                let function = read_function(dict, &color_space);
+                let function = read_function(dict, &color_space, cache);
                 let decode = dict
                     .get::<Array<'_>>(DECODE)?
                     .iter::<f32>()
@@ -222,12 +231,14 @@
                 let stream_data = stream.decoded().ok()?;
                 let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                 let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
-                let function = read_function(dict, &color_space);
+                let function = read_function(dict, &color_space, cache);
                 let decode = dict
                     .get::<Array<'_>>(DECODE)?
                     .iter::<f32>()
                     .collect::<Vec<_>>();
-                let vertices_per_row = dict.get::<u32>(VERTICES_PER_ROW)?;
+                // Tandem patch: fewer than two vertices per row are read as
+                // two (MuPDF); zero read rows without end.
+                let vertices_per_row = dict.get::<u32>(VERTICES_PER_ROW).unwrap_or(0).max(2);
 
                 let triangles = read_lattice_triangles(
                     stream_data.as_ref(),
@@ -249,7 +260,7 @@
                 let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                 let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
                 let bpf = dict.get::<u8>(BITS_PER_FLAG)?;
-                let function = read_function(dict, &color_space);
+                let function = read_function(dict, &color_space, cache);
                 let decode = dict
                     .get::<Array<'_>>(DECODE)?
                     .iter::<f32>()
@@ -272,7 +283,7 @@
                 let bp_coord = dict.get::<u8>(BITS_PER_COORDINATE)?;
                 let bp_comp = dict.get::<u8>(BITS_PER_COMPONENT)?;
                 let bpf = dict.get::<u8>(BITS_PER_FLAG)?;
-                let function = read_function(dict, &color_space);
+                let function = read_function(dict, &color_space, cache);
                 let decode = dict
                     .get::<Array<'_>>(DECODE)?
                     .iter::<f32>()
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
@@ -1026,11 +1079,15 @@
     )
 }
 
-fn read_function(dict: &Dict<'_>, color_space: &ColorSpace) -> Option<ShadingFunction> {
+fn read_function(
+    dict: &Dict<'_>,
+    color_space: &ColorSpace,
+    cache: &Cache,
+) -> Option<ShadingFunction> {
     if let Some(arr) = dict.get::<Array<'_>>(FUNCTION) {
         let arr: Option<SmallVec<_>> = arr
             .iter::<Object<'_>>()
-            .map(|o| Function::new(&o))
+            .map(|o| Function::new_with(&o, cache))
             .collect();
         let arr = arr?;
 
@@ -1042,7 +1099,7 @@
 
         Some(ShadingFunction::Multiple(arr))
     } else if let Some(obj) = dict.get::<Object<'_>>(FUNCTION) {
-        Some(ShadingFunction::Single(Function::new(&obj)?))
+        Some(ShadingFunction::Single(Function::new_with(&obj, cache)?))
     } else {
         None
     }
diff -ru a/src/types.rs b/src/types.rs
--- a/src/types.rs
+++ b/src/types.rs
@@ -47,6 +47,12 @@
         }
     }
 
+    /// Tandem patch: a key for what [`Self::with_stencil`] gives at a
+    /// target dimension.
+    pub fn shared_key(&self) -> Option<u128> {
+        self.image_xobject.shared_key()
+    }
+
     // These are hidden since clients are supposed to call get the
     // width/height from `LumaData` instead.
     #[doc(hidden)]
@@ -92,6 +98,13 @@
     /// This allows you to get access to the raw encoded image data, without doing any decoding.
     pub fn stream(&self) -> &Stream<'_> {
         self.0.stream()
+    }
+
+    /// Tandem patch: a key for what [`Self::with_rgba`] gives at a target
+    /// dimension, when that does not depend on the drawing state (a
+    /// transfer function, a colour space named in the resources).
+    pub fn shared_key(&self) -> Option<u128> {
+        self.0.shared_key()
     }
 
     // These are hidden since clients are supposed to call get the
Only in b/src/x_object/decode: budget.rs
diff -ru a/src/x_object/decode/image.rs b/src/x_object/decode/image.rs
--- a/src/x_object/decode/image.rs
+++ b/src/x_object/decode/image.rs
@@ -1,13 +1,18 @@
 use super::mask::decode_mask;
-use super::{DecodeContext, decode_context, decode_u8_samples, fix_image_length, unpack_samples};
+use super::{
+    DecodeContext, REDUCED_CACHE_BYTES, box_reduce, decode_context, decode_u8_samples,
+    fit_reduction, fix_image_length, over_budget, reduction, unpack_samples, unpacked_rows,
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
@@ -17,9 +22,108 @@
     obj: &ImageXObject<'_>,
     target_dimension: Option<(u32, u32)>,
 ) -> Option<DecodedImage> {
-    ImageDecoder::new(obj, target_dimension)?.decode()
+    // Tandem patch: an image drawn much smaller than its samples is reduced
+    // by whole-number box factors, and a decoded image is kept for the
+    // render (a page may draw one image many times) when it decodes alike
+    // wherever it is drawn.
+    let decode = || {
+        let mut decoded = ImageDecoder::new(obj, target_dimension)?.decode()?;
+        // A decoder that already reduced the samples (JPX skips resolution
+        // levels, an image past its budget is decoded reduced) leaves less.
+        let (width, height) = (decoded.image.width(), decoded.image.height());
+        let (kx, ky) = reduction(width, height, target_dimension);
+        if kx > 1 || ky > 1 {
+            decoded.image = reduce_image(decoded.image, kx, ky);
+        }
+        Some(decoded)
+    };
+    if !decodes_alike(obj) {
+        return decode();
+    }
+    let dict = obj.stream.dict();
+    let key = hash128(&(
+        b"image",
+        reduction(obj.width, obj.height, target_dimension),
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
 }
 
+/// Tandem patch: whether `obj` decodes alike wherever it is drawn: no
+/// transfer function, no colour space named in the resources.
+pub(crate) fn decodes_alike(obj: &ImageXObject<'_>) -> bool {
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
+    obj.transfer_function.is_none() && !named_space
+}
+
+/// Tandem patch: `image` averaged over `kx` × `ky` blocks (unchanged when
+/// its data does not hold its size).
+fn reduce_image(image: ImageData, kx: usize, ky: usize) -> ImageData {
+    let reduce = |data: &[u8], width: u32, height: u32, n: usize| {
+        let (w, h) = (width as usize, height as usize);
+        let (kx, ky) = (kx.clamp(1, w.max(1)), ky.clamp(1, h.max(1)));
+        Some((
+            box_reduce(data, w, h, n, kx, ky)?,
+            w.div_ceil(kx) as u32,
+            h.div_ceil(ky) as u32,
+        ))
+    };
+    let scale = |(sx, sy): (f32, f32), width: u32, height: u32, w: u32, h: u32| {
+        (
+            sx * width as f32 / w.max(1) as f32,
+            sy * height as f32 / h.max(1) as f32,
+        )
+    };
+    match image {
+        ImageData::Rgb(rgb) => {
+            let Some((data, width, height)) = reduce(&rgb.data, rgb.width, rgb.height, 3) else {
+                return ImageData::Rgb(rgb);
+            };
+            ImageData::Rgb(RgbData {
+                data,
+                scale_factors: scale(rgb.scale_factors, rgb.width, rgb.height, width, height),
+                width,
+                height,
+                interpolate: rgb.interpolate,
+            })
+        }
+        ImageData::Luma(luma) => {
+            let Some((data, width, height)) = reduce(&luma.data, luma.width, luma.height, 1)
+            else {
+                return ImageData::Luma(luma);
+            };
+            ImageData::Luma(LumaData {
+                data,
+                scale_factors: scale(luma.scale_factors, luma.width, luma.height, width, height),
+                width,
+                height,
+                interpolate: luma.interpolate,
+            })
+        }
+    }
+}
+
 struct ImageDecoder<'a, 'b> {
     obj: &'a ImageXObject<'b>,
     ctx: DecodeContext<'b>,
@@ -56,6 +160,18 @@
         if num_components == 0 {
             return None;
         }
+        // Tandem patch: samples within the image's budget are unpacked into
+        // `rows`; past it they are decoded reduced, a strip at a time.
+        let Some(rows) = unpacked_rows(
+            self.ctx.decoded.data.len(),
+            self.ctx.width,
+            self.ctx.height,
+            num_components,
+            self.ctx.bits_per_component,
+            self.obj.max_samples,
+        ) else {
+            return self.decode_image_reduced(num_components);
+        };
 
         let component_ranges = self.ctx.color_space.component_ranges();
         let default_decode = self
@@ -113,7 +229,7 @@
             let mut components = decode_u8_samples(
                 &self.ctx.decoded.data,
                 self.ctx.width,
-                self.ctx.height,
+                rows,
                 &self.ctx.color_space,
                 self.ctx.bits_per_component,
                 &self.ctx.decode_arr,
@@ -157,18 +273,134 @@
         Some(ImageData::Rgb(rgb_data))
     }
 
-    fn convert_to_rgb(&self, mut decoded: Vec<u8>) -> Option<RgbData> {
+    /// Tandem patch: an image whose samples pass its budget, decoded
+    /// reduced toward the size it is drawn at a strip of rows at a time, so
+    /// its samples are never held whole; `None` when it is not drawn
+    /// smaller or still passes the budget reduced.
+    fn decode_image_reduced(&mut self, n: usize) -> Option<ImageData> {
+        let (width, bits) = (self.ctx.width, self.ctx.bits_per_component);
+        let (kx, ky) = fit_reduction(width, self.ctx.height, self.target_dimension);
+        let row_bytes = (u64::from(width) * n as u64 * u64::from(bits))
+            .div_ceil(8)
+            .max(1);
+        let data = &self.ctx.decoded.data;
+        let rows = (data.len() as u64)
+            .div_ceil(row_bytes)
+            .min(u64::from(self.ctx.height));
+        let (out_width, out_height) = (
+            u64::from(width).div_ceil(kx as u64),
+            rows.div_ceil(ky as u64),
+        );
+        let strip = usize::try_from(row_bytes * ky as u64).ok();
+        let Some(strip) = strip.filter(|_| {
+            (kx > 1 || ky > 1) && out_width * out_height * n as u64 <= self.obj.max_samples
+        }) else {
+            return over_budget(self.obj);
+        };
+        let single = self
+            .obj
+            .transfer_function
+            .as_ref()
+            .is_none_or(|t| matches!(t, ActiveTransferFunction::Single(_)));
+        let key = self.color_key_mask.clone();
+        let (mut luma, mut out, mut key_out) = (None, Vec::new(), Vec::new());
+        let held = usize::try_from(row_bytes * rows).map_or(data.len(), |n| n.min(data.len()));
+        for strip in data[..held].chunks(strip) {
+            let strip_rows = (strip.len() as u64).div_ceil(row_bytes) as u32;
+            let samples = width as usize * strip_rows as usize * n;
+            if let Some(key) = &key {
+                let mut components = unpack_samples(strip, width, strip_rows, n, bits)?;
+                components.resize(samples, 0);
+                let keyed = key_mask(&components, n, key);
+                key_out.extend(box_reduce(&keyed, width as usize, strip_rows as usize, 1, kx, ky)?);
+            }
+            let mut components = decode_u8_samples(
+                strip,
+                width,
+                strip_rows,
+                &self.ctx.color_space,
+                bits,
+                &self.ctx.decode_arr,
+            )?;
+            components.resize(samples, 0);
+            let is_luma = match luma {
+                None => *luma.insert(single && self.ctx.color_space.to_luma(&mut components).is_some()),
+                Some(true) => {
+                    self.ctx.color_space.to_luma(&mut components)?;
+                    true
+                }
+                Some(false) => false,
+            };
+            let (mut pixels, channels) = if is_luma {
+                (components, 1)
+            } else {
+                (self.convert_rows(components, strip_rows)?, 3)
+            };
+            if let Some(transfer_function) = &self.obj.transfer_function {
+                transfer_function.apply_to(&mut pixels);
+            }
+            out.extend(box_reduce(
+                &pixels,
+                width as usize,
+                strip_rows as usize,
+                channels,
+                kx,
+                ky,
+            )?);
+        }
+        let (sx, sy) = self.ctx.scale_factors;
+        let scale_factors = (
+            sx * width as f32 / out_width.max(1) as f32,
+            sy * rows as f32 / out_height.max(1) as f32,
+        );
+        let (width, height) = (out_width as u32, out_height as u32);
+        let interpolate = self.obj.interpolate;
+        if key.is_some() {
+            self.decoded_color_key_mask = Some(LumaData {
+                data: key_out,
+                width,
+                height,
+                interpolate,
+                scale_factors,
+            });
+        }
+        Some(if luma == Some(true) {
+            ImageData::Luma(LumaData {
+                data: out,
+                width,
+                height,
+                interpolate,
+                scale_factors,
+            })
+        } else {
+            ImageData::Rgb(RgbData {
+                data: out,
+                width,
+                height,
+                interpolate,
+                scale_factors,
+            })
+        })
+    }
+
+    /// `rows` rows of decoded components as RGB.
+    fn convert_rows(&self, mut decoded: Vec<u8>, rows: u32) -> Option<Vec<u8>> {
         if self
             .ctx
             .color_space
             .convert_in_place(&mut decoded)
             .is_none()
         {
-            let mut output = vec![0; self.ctx.width as usize * self.ctx.height as usize * 3];
+            let mut output = vec![0; self.ctx.width as usize * rows as usize * 3];
             self.ctx.color_space.convert(&decoded, &mut output)?;
             decoded = output;
         }
+        Some(decoded)
+    }
 
+    fn convert_to_rgb(&self, decoded: Vec<u8>) -> Option<RgbData> {
+        let decoded = self.convert_rows(decoded, self.ctx.height)?;
+
         Some(RgbData {
             data: decoded,
             width: self.ctx.width,
@@ -185,9 +417,12 @@
     }
 
     fn decode_alpha(&mut self, image: &mut ImageData) -> Option<LumaData> {
+        // Tandem patch: and the image's own size (an image past its budget
+        // is decoded reduced).
         if let Some((alpha, matte_rgb)) = self.resolve_matte()
             && alpha.width == self.ctx.width
             && alpha.height == self.ctx.height
+            && (image.width(), image.height()) == (alpha.width, alpha.height)
         {
             unpremultiply(image, &alpha.data, &matte_rgb);
 
@@ -219,7 +454,7 @@
             .get::<Stream<'_>>(SMASK)
             .or_else(|| dict.get::<Stream<'_>>(MASK))
         {
-            let obj = ImageXObject::new_mask(&s_mask, &self.obj.warning_sink, &self.obj.cache)?;
+            let obj = ImageXObject::new_mask(&s_mask, self.obj)?;
 
             decode_mask(&obj, self.target_dimension).map(|decoded| decoded.luma)
         } else if self.color_key_mask.is_some() {
@@ -244,7 +479,7 @@
         let mut matte_rgb = [0_u8; 3];
         self.ctx.color_space.convert_values(&matte, &mut matte_rgb);
 
-        let mask_obj = ImageXObject::new_mask(&s_mask, &self.obj.warning_sink, &self.obj.cache)?;
+        let mask_obj = ImageXObject::new_mask(&s_mask, self.obj)?;
         let alpha = decode_mask(&mask_obj, self.target_dimension)?.luma;
 
         Some((alpha, matte_rgb))
@@ -253,27 +488,26 @@
     fn decode_color_key_mask(&self) -> Option<LumaData> {
         let color_key_mask = self.color_key_mask.as_deref()?;
         let num_components = self.ctx.color_space.num_components() as usize;
+        // Tandem patch: unpacked within the image's budget.
+        let Some(rows) = unpacked_rows(
+            self.ctx.decoded.data.len(),
+            self.ctx.width,
+            self.ctx.height,
+            num_components,
+            self.ctx.bits_per_component,
+            self.obj.max_samples,
+        ) else {
+            return over_budget(self.obj);
+        };
         let components = unpack_samples(
             &self.ctx.decoded.data,
             self.ctx.width,
-            self.ctx.height,
+            rows,
             num_components,
             self.ctx.bits_per_component,
         )?;
-        let mut data = Vec::with_capacity(self.ctx.width as usize * self.ctx.height as usize);
+        let mut data = key_mask(&components, num_components, color_key_mask);
 
-        for pixel in components.chunks_exact(num_components) {
-            let mut value = 0;
-
-            for (component, min_max) in pixel.iter().zip(color_key_mask.chunks_exact(2)) {
-                if *component > min_max[1] || *component < min_max[0] {
-                    value = 255;
-                }
-            }
-
-            data.push(value);
-        }
-
         let mut height = self.ctx.height;
         fix_image_length(&mut data, self.ctx.width, &mut height, 0, 1)?;
 
@@ -285,6 +519,26 @@
             scale_factors: self.ctx.scale_factors,
         })
     }
+}
+
+/// The colour key mask of `components` (`n` a pixel): 0 where every
+/// component is within its range of `key`, 255 elsewhere.
+fn key_mask(components: &[u16], n: usize, key: &[u16]) -> Vec<u8> {
+    let mut data = Vec::with_capacity(components.len() / n.max(1));
+
+    for pixel in components.chunks_exact(n) {
+        let mut value = 0;
+
+        for (component, min_max) in pixel.iter().zip(key.chunks_exact(2)) {
+            if *component > min_max[1] || *component < min_max[0] {
+                value = 255;
+            }
+        }
+
+        data.push(value);
+    }
+
+    data
 }
 
 fn unpremultiply(image: &mut ImageData, alpha: &[u8], matte_rgb: &[u8]) {
diff -ru a/src/x_object/decode/mask.rs b/src/x_object/decode/mask.rs
--- a/src/x_object/decode/mask.rs
+++ b/src/x_object/decode/mask.rs
@@ -1,6 +1,10 @@
-use super::{DecodeContext, decode_context, fix_image_length, unpack_samples};
+use super::{
+    DecodeContext, REDUCED_CACHE_BYTES, box_reduce, decode_context, fit_reduction,
+    fix_image_length, over_budget, reduction, unpack_samples, unpacked_rows,
+};
 use crate::LumaData;
 use crate::function::interpolate;
+use crate::util::hash128;
 use crate::x_object::image::{ImageKind, ImageXObject};
 use std::sync::LazyLock;
 
@@ -24,7 +28,51 @@
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
+                decode_mask_reduced(obj, ctx, invert, kx, ky)
+            },
+        )?;
+        return Some(DecodedMask { luma });
+    }
     let ctx = decode_context(obj, target_dimension)?;
+    // Tandem patch: samples within the budget are unpacked into `rows`; past
+    // it a mask drawn smaller is decoded reduced.
+    let Some(rows) = unpacked_rows(
+        ctx.decoded.data.len(),
+        ctx.width,
+        ctx.height,
+        ctx.color_space.num_components() as usize,
+        ctx.bits_per_component,
+        obj.max_samples,
+    ) else {
+        let (kx, ky) = fit_reduction(ctx.width, ctx.height, target_dimension);
+        if kx == 1 && ky == 1 {
+            return over_budget(obj);
+        }
+        let invert = obj.kind == ImageKind::StencilMask;
+        let luma = decode_mask_reduced(obj, ctx, invert, kx, ky)?;
+        return Some(DecodedMask { luma });
+    };
     let width = ctx.width;
     let scale_factors = ctx.scale_factors;
 
@@ -35,7 +83,7 @@
     // fully opaque. For stencil masks, it's the other way around: 1 means the
     // paint is visible, while 0 means it's invisible.
     let invert = obj.kind == ImageKind::StencilMask;
-    let (data, height) = decode_mask_data(ctx, invert)?;
+    let (data, height) = decode_mask_data(ctx, invert, rows)?;
 
     Some(DecodedMask {
         luma: LumaData {
@@ -48,7 +96,30 @@
     })
 }
 
-fn decode_mask_data(mut ctx: DecodeContext<'_>, invert: bool) -> Option<(Vec<u8>, u32)> {
+/// Tandem patch: the mask of `ctx` reduced by `kx` × `ky` box factors.
+fn decode_mask_reduced(
+    obj: &ImageXObject<'_>,
+    ctx: DecodeContext<'_>,
+    invert: bool,
+    kx: usize,
+    ky: usize,
+) -> Option<LumaData> {
+    let scale_factors = ctx.scale_factors;
+    let (data, width, height) = decode_mask_data_reduced(obj, ctx, invert, kx, ky)?;
+    Some(LumaData {
+        data,
+        width: width as u32,
+        height: height as u32,
+        interpolate: obj.interpolate,
+        scale_factors: (scale_factors.0 * kx as f32, scale_factors.1 * ky as f32),
+    })
+}
+
+fn decode_mask_data(
+    mut ctx: DecodeContext<'_>,
+    invert: bool,
+    rows: u32,
+) -> Option<(Vec<u8>, u32)> {
     let default_decode = ctx
         .color_space
         .default_decode_arr(ctx.bits_per_component as f32);
@@ -84,7 +155,7 @@
         let components = unpack_samples(
             &ctx.decoded.data,
             ctx.width,
-            ctx.height,
+            rows,
             num_components,
             ctx.bits_per_component,
         )?;
@@ -115,6 +186,101 @@
     Some((data, ctx.height))
 }
 
+/// Tandem patch: `decode_mask_data` reduced by `kx` × `ky` box factors,
+/// within the mask's budget; a one-bit mask is counted from its bits
+/// without expanding them, other masks are unpacked whole first.
+fn decode_mask_data_reduced(
+    obj: &ImageXObject<'_>,
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
+        let n = ctx.color_space.num_components() as usize;
+        let len = ctx.decoded.data.len();
+        let bits = ctx.bits_per_component;
+        let Some(rows) = unpacked_rows(len, ctx.width, ctx.height, n, bits, obj.max_samples)
+        else {
+            return over_budget(obj);
+        };
+        let (data, height) = decode_mask_data(ctx, invert, rows)?;
+        let height = height as usize;
+        let reduced = box_reduce(&data, width, height, 1, kx, ky)?;
+        return Some((reduced, width.div_ceil(kx), height.div_ceil(ky)));
+    }
+    let invert = invert ^ (ctx.decode_arr.as_slice() == inverted_default.as_slice());
+    let row_bytes = width.div_ceil(8).max(1);
+    let height = (ctx.height as usize).min(ctx.decoded.data.len().div_ceil(row_bytes));
+    let out_width = width.div_ceil(kx);
+    if (out_width as u64).saturating_mul(height.div_ceil(ky) as u64) > obj.max_samples {
+        return over_budget(obj);
+    }
+    let mut counts = vec![0usize; out_width];
+    let mut out = Vec::with_capacity(out_width * height.div_ceil(ky));
+    let mut rows = ctx.decoded.data.chunks(row_bytes).take(height);
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
@@ -125,7 +291,12 @@
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
@@ -1,7 +1,8 @@
+mod budget;
 mod image;
 mod mask;
 
-pub(crate) use image::{DecodedImage, decode_image};
+pub(crate) use image::{DecodedImage, decode_image, decodes_alike};
 pub(crate) use mask::{DecodedMask, decode_mask};
 
 use crate::InterpreterWarning;
@@ -11,7 +12,9 @@
 use hayro_syntax::bit_reader::BitReader;
 use hayro_syntax::object::Array;
 use hayro_syntax::object::dict::keys::*;
-use hayro_syntax::object::stream::{FilterResult, ImageColorSpace, ImageDecodeParams};
+use hayro_syntax::object::stream::{
+    DecodeFailure, FilterResult, ImageColorSpace, ImageDecodeParams,
+};
 use smallvec::SmallVec;
 use std::iter;
 
@@ -36,6 +39,12 @@
     let color_space = obj.color_space.clone();
     let is_indexed = obj.color_space.as_ref().is_some_and(|cs| cs.is_indexed());
 
+    // Tandem patch: decoders that size their output from declared sizes
+    // stay within the image's budget (a JPX image may skip more levels).
+    let Some(target_dimension) = budget::fit(obj, target_dimension) else {
+        return over_budget(obj);
+    };
+
     let decode_params = ImageDecodeParams {
         is_indexed,
         bpc: dict_bpc,
@@ -45,11 +54,15 @@
         height: obj.height,
     };
 
-    let decoded = obj
-        .stream
-        .decoded_image(&decode_params)
-        .map_err(|_| (obj.warning_sink)(InterpreterWarning::ImageDecodeFailure))
-        .ok()?;
+    // Tandem patch: data decoding past its limit is over budget.
+    let decoded = match obj.stream.decoded_image(&decode_params) {
+        Ok(decoded) => decoded,
+        Err(DecodeFailure::TooLarge) => return over_budget(obj),
+        Err(_) => {
+            (obj.warning_sink)(InterpreterWarning::ImageDecodeFailure);
+            return None;
+        }
+    };
 
     let (mut scale_x, mut scale_y) = (1.0, 1.0);
 
@@ -111,6 +124,115 @@
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
+/// Tandem patch: the largest whole-number box factors that keep an image of
+/// `width` × `height` samples at least `target` pixels, for an image whose
+/// samples pass its budget.
+pub(crate) fn fit_reduction(
+    width: u32,
+    height: u32,
+    target: Option<(u32, u32)>,
+) -> (usize, usize) {
+    target.map_or((1, 1), |(target_width, target_height)| {
+        let factor = |from: u32, to: u32| (from / to.max(1)).max(1) as usize;
+        (factor(width, target_width), factor(height, target_height))
+    })
+}
+
+/// Tandem patch: `data` (`width` × `height` pixels of `n` bytes) averaged
+/// over `kx` × `ky` blocks; partial blocks at the edges average what they
+/// hold. `None` unless `data` holds exactly those pixels.
+pub(crate) fn box_reduce(
+    data: &[u8],
+    width: usize,
+    height: usize,
+    n: usize,
+    kx: usize,
+    ky: usize,
+) -> Option<Vec<u8>> {
+    let row = width.checked_mul(n)?;
+    if width == 0 || n == 0 || row.checked_mul(height)? != data.len() {
+        return None;
+    }
+    let (kx, ky) = (kx.clamp(1, width), ky.clamp(1, height.max(1)));
+    let (out_width, out_height) = (width.div_ceil(kx), height.div_ceil(ky));
+    let mut out = Vec::with_capacity(out_width * out_height * n);
+    let mut sums = vec![0u64; out_width * n];
+    for block_rows in data.chunks(row * ky) {
+        sums.iter_mut().for_each(|s| *s = 0);
+        let row_count = (block_rows.len() / row) as u64;
+        for row in block_rows.chunks_exact(row) {
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
+            let count = ((bx * kx + kx).min(width) - bx * kx) as u64 * row_count;
+            for s in &sums[bx * n..(bx + 1) * n] {
+                out.push(((s + count / 2) / count) as u8);
+            }
+        }
+    }
+    Some(out)
+}
+
+/// Tandem patch: the rows an image's samples are unpacked into within
+/// `budget` samples: the declared `height` for 1 to 7 and 9 to 15 bits per
+/// component (missing data read as 0, as MuPDF pads it), else the rows its
+/// `len` bytes hold; `None` when even those pass the budget.
+pub(crate) fn unpacked_rows(
+    len: usize,
+    width: u32,
+    height: u32,
+    n: usize,
+    bits: u8,
+    budget: u64,
+) -> Option<u32> {
+    let row = u64::from(width).saturating_mul(n as u64);
+    if matches!(bits, 1..=7 | 9..=15) && row.saturating_mul(u64::from(height)) <= budget {
+        return Some(height);
+    }
+    let row_bytes = row.saturating_mul(u64::from(bits)).div_ceil(8).max(1);
+    let held = (len as u64).div_ceil(row_bytes).min(u64::from(height));
+    (row.saturating_mul(held) <= budget).then_some(held as u32)
+}
+
+/// Tandem patch: `None`, after telling the warning sink that `obj` passed
+/// its budget.
+pub(crate) fn over_budget<T>(obj: &ImageXObject<'_>) -> Option<T> {
+    warn!("image exceeds its decoding budget");
+    (obj.warning_sink)(InterpreterWarning::BudgetExceeded);
+    None
+}
+
 #[must_use]
 fn fix_image_length<T: Copy>(
     image: &mut Vec<T>,
@@ -119,17 +241,24 @@
     filler: T,
     num_components: usize,
 ) -> Option<()> {
-    let row_len = width as usize * num_components;
+    // Tandem patch: sizes in u64 (on wasm32 `row_len * height` wrapped and
+    // cut the data to nothing while the height stayed); a row longer than
+    // the largest budget is refused rather than padded.
+    let row_len = u64::from(width) * num_components as u64;
+    if row_len == 0 || row_len > crate::MAX_IMAGE_SAMPLES.max(image.len() as u64) {
+        return None;
+    }
+    let len = image.len() as u64;
 
-    if (row_len * *height as usize) <= image.len() {
+    if row_len.saturating_mul(u64::from(*height)) <= len {
         // Too much data (or just the right amount), truncate it.
-        image.truncate(row_len * *height as usize);
+        image.truncate((row_len * u64::from(*height)) as usize);
     } else {
         // Too little data, adapt the height and pad.
-        *height = image.len().div_ceil(row_len) as u32;
+        *height = len.div_ceil(row_len) as u32;
 
-        if !image.len().is_multiple_of(row_len) {
-            image.extend(iter::repeat_n(filler, row_len - (image.len() % row_len)));
+        if !len.is_multiple_of(row_len) {
+            image.extend(iter::repeat_n(filler, (row_len - len % row_len) as usize));
         }
     }
 
@@ -150,7 +279,6 @@
 ) -> Option<Vec<u8>> {
     let source_max = 2.0_f32.powi(bits_per_component as i32) - 1.0;
     let num_components = color_space.num_components() as usize;
-    let capacity = width as usize * height as usize * num_components;
     let ranges = color_space.component_ranges();
     let indexed_hival = color_space.indexed_hival();
 
@@ -174,7 +302,7 @@
 
     match bits_per_component {
         1..8 | 9..16 => {
-            let mut buf = Vec::with_capacity(capacity);
+            let mut buf = Vec::with_capacity(samples(width, height, num_components)?);
             for_each_sample(
                 data,
                 width,
@@ -217,11 +345,9 @@
     num_components: usize,
     bits_per_component: u8,
 ) -> Option<Vec<u16>> {
-    let capacity = width as usize * height as usize * num_components;
-
     match bits_per_component {
         1..8 | 9..16 => {
-            let mut buf = Vec::with_capacity(capacity);
+            let mut buf = Vec::with_capacity(samples(width, height, num_components)?);
             for_each_sample(
                 data,
                 width,
@@ -247,6 +373,15 @@
             None
         }
     }
+}
+
+/// Tandem patch: `width` × `height` × `n` as a `usize` (callers bound it
+/// with `unpacked_rows`).
+fn samples(width: u32, height: u32, n: usize) -> Option<usize> {
+    let samples = u64::from(width)
+        .checked_mul(u64::from(height))?
+        .checked_mul(n as u64)?;
+    usize::try_from(samples).ok()
 }
 
 fn for_each_sample(
diff -ru a/src/x_object/image.rs b/src/x_object/image.rs
--- a/src/x_object/image.rs
+++ b/src/x_object/image.rs
@@ -1,4 +1,4 @@
-use super::decode::{DecodedImage, DecodedMask, decode_image, decode_mask};
+use super::decode::{DecodedImage, DecodedMask, decode_image, decode_mask, decodes_alike};
 use super::xobject_oc;
 use crate::WarningSinkFn;
 use crate::cache::Cache;
@@ -6,6 +6,7 @@
 use crate::context::Context;
 use crate::device::Device;
 use crate::interpret::state::ActiveTransferFunction;
+use crate::util::hash128;
 use crate::{BlendMode, CacheKey, Image, ImageDrawProps, RasterImage, StencilImage};
 use hayro_syntax::object::dict::keys::*;
 use hayro_syntax::object::{Name, Object, Stream};
@@ -22,6 +23,8 @@
     pub(crate) stream: Stream<'a>,
     pub(crate) transfer_function: Option<ActiveTransferFunction>,
     pub(crate) warning_sink: WarningSinkFn,
+    /// Tandem patch: `InterpreterSettings::max_image_samples`.
+    pub(crate) max_samples: u64,
 }
 
 #[derive(Clone, Copy, PartialEq, Eq)]
@@ -44,6 +47,7 @@
         warning_sink: &WarningSinkFn,
         cache: &Cache,
         transfer_function: Option<ActiveTransferFunction>,
+        max_samples: u64,
     ) -> Option<Self> {
         Self::new_inner(
             stream,
@@ -53,14 +57,26 @@
             ImageKind::Image,
             transfer_function,
         )
+        .map(|image| Self {
+            max_samples,
+            ..image
+        })
     }
 
-    pub(crate) fn new_mask(
-        stream: &Stream<'a>,
-        warning_sink: &WarningSinkFn,
-        cache: &Cache,
-    ) -> Option<Self> {
-        Self::new_inner(stream, |_| None, warning_sink, cache, ImageKind::Mask, None)
+    /// Tandem patch: the mask's budget is its image's.
+    pub(crate) fn new_mask(stream: &Stream<'a>, image: &Self) -> Option<Self> {
+        Self::new_inner(
+            stream,
+            |_| None,
+            &image.warning_sink,
+            &image.cache,
+            ImageKind::Mask,
+            None,
+        )
+        .map(|mask| Self {
+            max_samples: image.max_samples,
+            ..mask
+        })
     }
 
     fn new_inner(
@@ -125,6 +141,7 @@
             interpolate,
             stream: stream.clone(),
             kind,
+            max_samples: crate::MAX_IMAGE_SAMPLES,
         })
     }
 
@@ -215,6 +232,16 @@
         }
 
         decode_image(self, target_dimension)
+    }
+
+    /// Tandem patch: a key for what `self` decodes to at a target
+    /// dimension (its dictionary and data), when that does not depend on
+    /// the drawing state.
+    pub(crate) fn shared_key(&self) -> Option<u128> {
+        (self.kind.is_mask() || decodes_alike(self)).then(|| {
+            let data = self.stream.raw_data();
+            hash128(&(self.stream.dict().data(), data.as_ref()))
+        })
     }
 
     pub(crate) fn width(&self) -> u32 {
diff -ru a/src/x_object/mod.rs b/src/x_object/mod.rs
--- a/src/x_object/mod.rs
+++ b/src/x_object/mod.rs
@@ -31,6 +31,7 @@
         warning_sink: &WarningSinkFn,
         cache: &Cache,
         transfer_function: Option<ActiveTransferFunction>,
+        max_samples: u64,
     ) -> Option<Self> {
         let dict = stream.dict();
         match dict.get::<Name<'_>>(SUBTYPE)?.deref() {
@@ -40,6 +41,7 @@
                 warning_sink,
                 cache,
                 transfer_function,
+                max_samples,
             )?)),
             FORM => Some(Self::FormXObject(FormXObject::new(stream)?)),
             _ => None,
diff -ru a/src/x_object/soft_mask.rs b/src/x_object/soft_mask.rs
--- a/src/x_object/soft_mask.rs
+++ b/src/x_object/soft_mask.rs
@@ -92,7 +92,7 @@
         )?;
         let transfer_function = dict
             .get::<Object<'_>>(TR)
-            .and_then(|o| Function::new(&o))
+            .and_then(|o| Function::new_with(&o, &context.interpreter_cache.object_cache))
             .map(TransferFunction::new);
         let (mask_type, background) = match dict.get::<Name<'_>>(S)?.deref() {
             LUMINOSITY => {
```
