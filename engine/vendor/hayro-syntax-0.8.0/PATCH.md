# hayro-syntax 0.8.0, patched for Tandem PDF

This directory is hayro-syntax 0.8.0 as published on crates.io (`.crate`
checksum `6cdc818d8d265dcb42081d488f7599df4f73fb79e9d347ecedcd3c23809d2026`,
the one `Cargo.lock` pinned before), with the patch below. It is wired in
with `[patch.crates-io]` in `engine/Cargo.toml` and `engine/bridge/Cargo.toml`.
Licence: Apache-2.0 OR MIT (`LICENSE-APACHE`, `LICENSE-MIT`, unchanged). Why a
patch and not a guard in the engine: hayro-syntax decodes every stream hayro
reads (content, images, fonts, functions, profiles), and the engine's own
decoders cannot tell beforehand what size hayro's will reach (hayro tries
zlib, raw deflate and a lenient inflater in turn). Every change is marked
`Tandem patch` in the source. Design notes and measurements:
`engine/design/app-accept-integration.md`; tests: `engine/tests/view_hostile.rs`
and, for the limits on the WASM build, `tests/rust-engine-bounds.test.mjs`.

## Changes

- `src/object/stream.rs`, `src/filter/*.rs`, decoding limits: Flate (zlib,
  raw deflate and the fallback inflater), LZW, run-length and ASCII85 stop
  once their output passes the stream's limit, and that filter fails with
  the new `DecodeFailure::TooLarge`. The limit is `MAX_DECODED_BYTES`
  (2^27, 128 MiB) for one filter of a stream, or for an image what its
  declared size takes (rows of width × components × bits, a predictor byte
  each, an eighth more) within `MAX_IMAGE_DECODED_BYTES` (2^29). Output
  buffers double only up to the limit (`filter::reserve_within`). hayro
  decoded without a limit: 1,871 bytes of nested Flate content decoded to
  768 MB and took 2 GB of WASM memory (wasm32 aborts at 4 GiB).
- `src/page.rs`: a page's content streams, alone and together, are held to
  `MAX_DECODED_BYTES`; past it the page has no content and
  `Page::content_too_large` says so (the engine then refuses the page). One
  stream listed many times in /Contents was decoded and copied each time.
- `src/filter/lzw_flate.rs`, predictors: with a row of no bytes (`/Columns 0`
  or `/Colors 0`) the data passes unpredicted, as MuPDF 1.28.1 reads it
  (hayro divided by zero: a panic, so a WASM trap, from any Flate stream with
  such parameters), data shorter than one
  row decodes to nothing before the zero row is allocated (a large
  `/Columns` allocated gigabytes first), and the bits per pixel are a usize
  (32 colors of 16 bits wrapped a u8).

## Diff against the published crate

```diff
diff -ru a/src/filter/ascii_85.rs b/src/filter/ascii_85.rs
--- a/src/filter/ascii_85.rs
+++ b/src/filter/ascii_85.rs
@@ -4,7 +4,8 @@
 use crate::trivia::is_white_space_character;
 use alloc::vec::Vec;
 
-pub(crate) fn decode(data: &[u8]) -> Option<Vec<u8>> {
+// Tandem patch: decoding stops once the output passes `limit`.
+pub(crate) fn decode(data: &[u8], limit: usize) -> Option<Vec<u8>> {
     const POW_85: [u32; 5] = [52200625, 614125, 7225, 85, 1];
 
     let mut reader = Reader::new(data);
@@ -59,6 +60,11 @@
     let mut group = Vec::with_capacity(5);
 
     loop {
+        if decoded.len() > limit {
+            return Some(decoded);
+        }
+        crate::filter::reserve_within(&mut decoded, 4, limit);
+
         let Some(b) = read_byte() else {
             // Be lenient and accept what we have (see PDFBOX-5910).
             flush_group(&mut group, &mut decoded)?;
diff -ru a/src/filter/lzw_flate.rs b/src/filter/lzw_flate.rs
--- a/src/filter/lzw_flate.rs
+++ b/src/filter/lzw_flate.rs
@@ -10,37 +10,55 @@
     use crate::filter::lzw_flate::{PredictorParams, apply_predictor};
     use crate::object::Dict;
 
+    // Tandem patch: decoding stops once the output passes `limit` (the
+    // output is returned then, longer than `limit`, without a predictor).
     #[cfg(feature = "unsafe")]
-    pub(crate) fn decode(data: &[u8], params: &Dict<'_>) -> Option<Vec<u8>> {
+    pub(crate) fn decode(data: &[u8], params: &Dict<'_>, limit: usize) -> Option<Vec<u8>> {
         use flate2::read::{DeflateDecoder, ZlibDecoder};
         use std::io::Read;
 
-        fn zlib_stream(data: &[u8]) -> Option<Vec<u8>> {
-            let mut decoder = ZlibDecoder::new(data);
+        // At most `limit + 1` bytes, the buffer growing within them.
+        let read_limited = |decoder: &mut dyn Read| -> Option<Vec<u8>> {
             let mut result = Vec::new();
-            decoder.read_to_end(&mut result).ok().map(|_| result)
-        }
+            let mut chunk = vec![0; 1 << 16];
+            loop {
+                let room = (limit.saturating_add(1) - result.len()).min(chunk.len());
+                let read = decoder.read(&mut chunk[..room]).ok()?;
+                if read == 0 {
+                    return Some(result);
+                }
+                crate::filter::reserve_within(&mut result, read, limit);
+                result.extend_from_slice(&chunk[..read]);
+                if result.len() > limit {
+                    return Some(result);
+                }
+            }
+        };
 
-        fn deflate_stream(data: &[u8]) -> Option<Vec<u8>> {
-            let mut decoder = DeflateDecoder::new(data);
-            let mut result = Vec::new();
-            decoder.read_to_end(&mut result).ok().map(|_| result)
-        }
+        let zlib_stream = |data: &[u8]| read_limited(&mut ZlibDecoder::new(data));
 
+        let deflate_stream = |data: &[u8]| read_limited(&mut DeflateDecoder::new(data));
+
         let decoded = zlib_stream(data)
             .or_else(|| deflate_stream(data))
             .or_else(|| {
                 warn!("flate stream is broken, decoding with fallback");
 
-                fallback::decode(data)
+                fallback::decode(data, limit)
             })?;
+        if decoded.len() > limit {
+            return Some(decoded);
+        }
         let params = PredictorParams::from_params(params);
         apply_predictor(decoded, &params)
     }
 
     #[cfg(not(feature = "unsafe"))]
-    pub(crate) fn decode(data: &[u8], params: &Dict<'_>) -> Option<Vec<u8>> {
-        let decoded = fallback::decode(data)?;
+    pub(crate) fn decode(data: &[u8], params: &Dict<'_>, limit: usize) -> Option<Vec<u8>> {
+        let decoded = fallback::decode(data, limit)?;
+        if decoded.len() > limit {
+            return Some(decoded);
+        }
         let params = PredictorParams::from_params(params);
         apply_predictor(decoded, &params)
     }
@@ -51,11 +69,11 @@
         use alloc::vec;
         use alloc::vec::Vec;
 
-        pub(crate) fn decode(data: &[u8]) -> Option<Vec<u8>> {
-            flate_decode(data)
+        pub(crate) fn decode(data: &[u8], limit: usize) -> Option<Vec<u8>> {
+            flate_decode(data, limit)
         }
 
-        fn flate_decode(data: &[u8]) -> Option<Vec<u8>> {
+        fn flate_decode(data: &[u8], limit: usize) -> Option<Vec<u8>> {
             if data.len() >= 2 {
                 let cmf = data[0];
                 let flg = data[1];
@@ -64,12 +82,12 @@
                     && ((cmf as u16) << 8 | flg as u16).is_multiple_of(31)
                     && (flg & 0x20) == 0
                 {
-                    let mut stream = FlateStream::new(&data[2..]);
+                    let mut stream = FlateStream::new(&data[2..], limit);
                     return stream.decode();
                 }
             }
 
-            let mut stream = FlateStream::new(data);
+            let mut stream = FlateStream::new(data, limit);
             stream.decode()
         }
 
@@ -80,10 +98,12 @@
             code_size: u8,
             output: Vec<u8>,
             eof: bool,
+            // Tandem patch: decoding stops once the output passes it.
+            limit: usize,
         }
 
         impl<'a> FlateStream<'a> {
-            fn new(data: &'a [u8]) -> Self {
+            fn new(data: &'a [u8], limit: usize) -> Self {
                 FlateStream {
                     data,
                     pos: 0,
@@ -91,11 +111,12 @@
                     code_size: 0,
                     output: Vec::new(),
                     eof: false,
+                    limit,
                 }
             }
 
             fn decode(&mut self) -> Option<Vec<u8>> {
-                while !self.eof && self.pos < self.data.len() {
+                while !self.eof && self.pos < self.data.len() && self.output.len() <= self.limit {
                     self.read_block();
                 }
 
@@ -253,6 +274,7 @@
                     }
                 } else {
                     let block = self.get_bytes(block_len as usize);
+                    crate::filter::reserve_within(&mut self.output, block.len(), self.limit);
                     self.output.extend_from_slice(&block);
                     if block.len() < block_len as usize {
                         self.eof = true;
@@ -274,6 +296,13 @@
                 };
 
                 loop {
+                    // Tandem patch: at most a 258-byte copy past the limit,
+                    // the output growing within it.
+                    if self.output.len() > self.limit {
+                        return;
+                    }
+                    crate::filter::reserve_within(&mut self.output, 258, self.limit);
+
                     let code1 = match self.get_code(&lit_code_table) {
                         Some(c) => c,
                         None => {
@@ -570,10 +599,15 @@
     use alloc::vec::Vec;
 
     /// Decode a LZW-encoded stream.
-    pub(crate) fn decode(data: &[u8], params: &Dict<'_>) -> Option<Vec<u8>> {
+    // Tandem patch: decoding stops once the output passes `limit` (the
+    // output is returned then, longer than `limit`, without a predictor).
+    pub(crate) fn decode(data: &[u8], params: &Dict<'_>, limit: usize) -> Option<Vec<u8>> {
         let params = PredictorParams::from_params(params);
 
-        let decoded = decode_impl(data, params.early_change)?;
+        let decoded = decode_impl(data, params.early_change, limit)?;
+        if decoded.len() > limit {
+            return Some(decoded);
+        }
 
         apply_predictor(decoded, &params)
     }
@@ -583,7 +617,7 @@
     const MAX_ENTRIES: usize = 4096;
     const INITIAL_SIZE: u16 = 258;
 
-    fn decode_impl(data: &[u8], early_change: bool) -> Option<Vec<u8>> {
+    fn decode_impl(data: &[u8], early_change: bool, limit: usize) -> Option<Vec<u8>> {
         let mut table = Table::new(early_change);
         let mut bit_size = table.code_length();
         let mut reader = BitReader::new(data);
@@ -591,6 +625,13 @@
         let mut prev = None;
 
         loop {
+            // Tandem patch: at most a 4,096-byte entry past the limit, the
+            // output growing within it.
+            if decoded.len() > limit {
+                return Some(decoded);
+            }
+            crate::filter::reserve_within(&mut decoded, 4096, limit);
+
             let next = match reader.read(bit_size) {
                 Some(code) => code as usize,
                 None => {
@@ -717,12 +758,13 @@
 }
 
 impl PredictorParams {
-    fn bits_per_pixel(&self) -> u8 {
-        self.bits_per_component * self.colors
+    // Tandem patch: sizes in usize, checked; the u8 product wrapped.
+    fn bits_per_pixel(&self) -> usize {
+        usize::from(self.bits_per_component) * usize::from(self.colors)
     }
 
-    fn row_length_in_bytes(&self) -> usize {
-        (self.columns * self.bits_per_pixel() as usize).div_ceil(8)
+    fn row_length_in_bytes(&self) -> Option<usize> {
+        Some(self.columns.checked_mul(self.bits_per_pixel())?.div_ceil(8))
     }
 }
 
@@ -756,7 +798,7 @@
         i => {
             let is_png_predictor = i >= 10;
 
-            let row_len = params.row_length_in_bytes();
+            let row_len = params.row_length_in_bytes()?;
 
             let total_row_len = if is_png_predictor {
                 // + 1 Because each row must start with the predictor that is used for PNG predictors.
@@ -765,6 +807,17 @@
                 row_len
             };
 
+            // Tandem patch: a row of no bytes (/Columns or /Colors 0) has
+            // nothing to predict, and the data passes unchanged, as MuPDF
+            // 1.28.1 reads it (hayro divided by zero); without a whole row
+            // there is no output (hayro allocated a zero row of the declared
+            // /Columns first: gigabytes for a large one).
+            if row_len == 0 {
+                return Some(data);
+            }
+            if data.len() < total_row_len {
+                return Some(Vec::new());
+            }
             let num_rows = data.len() / total_row_len;
 
             if !matches!(params.bits_per_component, 1 | 2 | 4 | 8 | 16) {
@@ -774,10 +827,7 @@
             }
 
             let (bit_size, chunk_len) = if is_png_predictor {
-                (
-                    8,
-                    (params.colors * params.bits_per_component).div_ceil(8) as usize,
-                )
+                (8, params.bits_per_pixel().div_ceil(8))
             } else {
                 (params.bits_per_component, params.colors as usize)
             };
@@ -1033,7 +1083,7 @@
     #[test]
     fn decode_lzw() {
         let input = [0x80, 0x0B, 0x60, 0x50, 0x22, 0x0C, 0x0C, 0x85, 0x01];
-        let decoded = lzw::decode(&input, &Dict::default()).unwrap();
+        let decoded = lzw::decode(&input, &Dict::default(), usize::MAX).unwrap();
 
         assert_eq!(decoded, vec![45, 45, 45, 45, 45, 65, 45, 45, 45, 66]);
     }
@@ -1044,7 +1094,7 @@
             0x78, 0x9c, 0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x7, 0x0, 0x5, 0x8c, 0x1, 0xf5,
         ];
 
-        let decoded = flate::decode(&input, &Dict::default()).unwrap();
+        let decoded = flate::decode(&input, &Dict::default(), usize::MAX).unwrap();
         assert_eq!(decoded, b"Hello");
     }
 
@@ -1052,7 +1102,7 @@
     fn decode_flate() {
         let input = [0xf3, 0x48, 0xcd, 0xc9, 0xc9, 0x7, 0x0];
 
-        let decoded = flate::decode(&input, &Dict::default()).unwrap();
+        let decoded = flate::decode(&input, &Dict::default(), usize::MAX).unwrap();
         assert_eq!(decoded, b"Hello");
     }
     
diff -ru a/src/filter/mod.rs b/src/filter/mod.rs
--- a/src/filter/mod.rs
+++ b/src/filter/mod.rs
@@ -18,8 +18,24 @@
 use crate::object::Name;
 use crate::object::dict::keys::*;
 use crate::object::stream::{DecodeFailure, FilterResult, ImageDecodeParams};
+use alloc::vec::Vec;
 use core::ops::Deref;
 
+/// Tandem patch: room for `extra` more bytes in `out`, which grows by
+/// doubling but not past `limit + 1` (a decoder stops once past `limit`), so
+/// output near the limit does not double its buffer past it.
+pub(crate) fn reserve_within(out: &mut Vec<u8>, extra: usize, limit: usize) {
+    let wanted = out.len().saturating_add(extra);
+    if wanted > out.capacity() {
+        let target = out
+            .capacity()
+            .saturating_mul(2)
+            .min(limit.saturating_add(1))
+            .max(wanted);
+        out.reserve_exact(target - out.len());
+    }
+}
+
 /// A data filter.
 #[derive(Debug, Copy, Clone, PartialEq, Eq)]
 pub enum Filter {
@@ -86,23 +102,21 @@
         data: &[u8],
         params: &Dict<'_>,
         #[cfg_attr(not(feature = "images"), allow(unused))] image_params: &ImageDecodeParams,
+        limit: usize,
     ) -> Result<FilterResult<'static>, DecodeFailure> {
+        // Tandem patch: these filters stop once their output passes `limit`
+        // (they return it then, longer than `limit`), and that is a failure.
+        let limited = |data: Option<Vec<u8>>| match data {
+            Some(data) if data.len() > limit => Err(DecodeFailure::TooLarge),
+            Some(data) => Ok(FilterResult::from_data(data)),
+            None => Err(DecodeFailure::StreamDecode),
+        };
         let res = match self {
-            Self::AsciiHexDecode => ascii_hex::decode(data)
-                .map(FilterResult::from_data)
-                .ok_or(DecodeFailure::StreamDecode),
-            Self::Ascii85Decode => ascii_85::decode(data)
-                .map(FilterResult::from_data)
-                .ok_or(DecodeFailure::StreamDecode),
-            Self::RunLengthDecode => run_length::decode(data)
-                .map(FilterResult::from_data)
-                .ok_or(DecodeFailure::StreamDecode),
-            Self::LzwDecode => lzw_flate::lzw::decode(data, params)
-                .map(FilterResult::from_data)
-                .ok_or(DecodeFailure::StreamDecode),
-            Self::FlateDecode => lzw_flate::flate::decode(data, params)
-                .map(FilterResult::from_data)
-                .ok_or(DecodeFailure::StreamDecode),
+            Self::AsciiHexDecode => limited(ascii_hex::decode(data)),
+            Self::Ascii85Decode => limited(ascii_85::decode(data, limit)),
+            Self::RunLengthDecode => limited(run_length::decode(data, limit)),
+            Self::LzwDecode => limited(lzw_flate::lzw::decode(data, params, limit)),
+            Self::FlateDecode => limited(lzw_flate::flate::decode(data, params, limit)),
             #[cfg(feature = "images")]
             Self::DctDecode => {
                 dct::decode(data, params, image_params).ok_or(DecodeFailure::ImageDecode)
diff -ru a/src/filter/run_length.rs b/src/filter/run_length.rs
--- a/src/filter/run_length.rs
+++ b/src/filter/run_length.rs
@@ -2,11 +2,13 @@
 use alloc::vec;
 use alloc::vec::Vec;
 
-pub(crate) fn decode(data: &[u8]) -> Option<Vec<u8>> {
+// Tandem patch: decoding stops once the output passes `limit`.
+pub(crate) fn decode(data: &[u8], limit: usize) -> Option<Vec<u8>> {
     let mut reader = Reader::new(data);
     let mut decoded = vec![];
 
-    loop {
+    while decoded.len() <= limit {
+        crate::filter::reserve_within(&mut decoded, 128, limit);
         let length = reader.read_byte()?;
 
         match length {
@@ -37,7 +39,7 @@
     fn run_length() {
         let input = vec![4, 10, 11, 12, 13, 14, 253, 3, 128];
         assert_eq!(
-            decode(&input).unwrap(),
+            decode(&input, usize::MAX).unwrap(),
             vec![10, 11, 12, 13, 14, 3, 3, 3, 3]
         );
     }
diff -ru a/src/object/stream.rs b/src/object/stream.rs
--- a/src/object/stream.rs
+++ b/src/object/stream.rs
@@ -185,6 +185,7 @@
                 current.as_ref().map(|c| c.data.as_ref()).unwrap_or(&data),
                 params,
                 image_params,
+                decode_limit(image_params),
             )?;
             current = Some(new);
         }
@@ -250,6 +251,33 @@
     Decryption,
     /// An unknown failure occurred.
     Unknown,
+    /// Tandem patch: a filter's output passed the stream's decoding limit
+    /// ([`MAX_DECODED_BYTES`], or for an image [`MAX_IMAGE_DECODED_BYTES`]).
+    TooLarge,
+}
+
+// Tandem patch: decoding limits. Flate, LZW, run-length and ASCII85 stop
+// once their output passes the limit; a few bytes of nested Flate decoded
+// to gigabytes, and on wasm32 an allocation failure aborts.
+/// The bytes one filter of a stream may decode to; a page's concatenated
+/// content is held to it too.
+pub const MAX_DECODED_BYTES: usize = 1 << 27;
+/// The bytes one filter of an image stream may decode to, when its declared
+/// size needs more than [`MAX_DECODED_BYTES`].
+pub const MAX_IMAGE_DECODED_BYTES: usize = 1 << 29;
+
+/// Tandem patch: the limit for one filter of a stream decoded with
+/// `params`: [`MAX_DECODED_BYTES`], or for an image what its declared size
+/// takes (rows of `width × components × bits` plus a predictor byte each,
+/// and an eighth more), within [`MAX_IMAGE_DECODED_BYTES`].
+fn decode_limit(params: &ImageDecodeParams) -> usize {
+    let bits = u64::from(params.bpc.unwrap_or(8)) * u64::from(params.num_components.unwrap_or(4));
+    let row = (u64::from(params.width) * bits).div_ceil(8) + 1;
+    let declared = row.saturating_mul(u64::from(params.height));
+    let wanted = declared.saturating_add(declared / 8);
+    usize::try_from(wanted.min(MAX_IMAGE_DECODED_BYTES as u64))
+        .unwrap_or(MAX_IMAGE_DECODED_BYTES)
+        .max(MAX_DECODED_BYTES)
 }
 
 /// An image color space.
diff -ru a/src/page.rs b/src/page.rs
--- a/src/page.rs
+++ b/src/page.rs
@@ -8,6 +8,7 @@
 use crate::object::Stream;
 use crate::object::array::ArrayIter;
 use crate::object::dict::keys::*;
+use crate::object::stream::{DecodeFailure, MAX_DECODED_BYTES};
 use crate::object::{MaybeRef, Object};
 use crate::reader::ReaderContext;
 use crate::sync::{FxHashSet, OnceLock};
@@ -191,7 +192,8 @@
     media_box: Rect,
     crop_box: Rect,
     rotation: Rotation,
-    page_streams: OnceLock<Option<Vec<u8>>>,
+    // Tandem patch: with whether the content passed `MAX_DECODED_BYTES`.
+    page_streams: OnceLock<(Option<Vec<u8>>, bool)>,
     resources: Resources<'a>,
     ctx: ReaderContext<'a>,
 }
@@ -253,36 +255,61 @@
     }
 
     /// Return the decoded content stream of the page.
+    ///
+    /// Tandem patch: `None` when the content decodes past
+    /// [`MAX_DECODED_BYTES`] (see [`Self::content_too_large`]).
     pub fn page_stream(&self) -> Option<&[u8]> {
-        let convert_single = |s: Stream<'_>| {
-            let data = s.decoded().ok()?;
-            Some(data.to_vec())
-        };
+        self.page_streams().0.as_deref()
+    }
 
-        self.page_streams
-            .get_or_init(|| {
-                if let Some(stream) = self.inner.get::<Stream<'_>>(CONTENTS) {
-                    convert_single(stream)
-                } else if let Some(array) = self.inner.get::<Array<'_>>(CONTENTS) {
-                    let streams = array.iter::<Stream<'_>>().flat_map(convert_single);
+    /// Tandem patch: whether the page's content, one stream or all of them
+    /// together, decodes past [`MAX_DECODED_BYTES`]; the page then has no
+    /// content (`page_stream` is `None`).
+    pub fn content_too_large(&self) -> bool {
+        self.page_streams().1
+    }
 
-                    let mut collected = vec![];
+    fn page_streams(&self) -> &(Option<Vec<u8>>, bool) {
+        self.page_streams.get_or_init(|| {
+            // Tandem patch: every stream and their concatenation are held to
+            // `MAX_DECODED_BYTES`; one large stream listed many times in
+            // /Contents was decoded and copied once per listing.
+            const TOO_LARGE: (Option<Vec<u8>>, bool) = (None, true);
+            if let Some(stream) = self.inner.get::<Stream<'_>>(CONTENTS) {
+                match stream.decoded() {
+                    Ok(data) => (Some(data.into_owned()), false),
+                    Err(DecodeFailure::TooLarge) => TOO_LARGE,
+                    Err(_) => (None, false),
+                }
+            } else if let Some(array) = self.inner.get::<Array<'_>>(CONTENTS) {
+                let mut collected = vec![];
 
-                    for stream in streams {
-                        collected.extend(stream);
-                        // Streams must have at least one whitespace in-between.
-                        collected.push(b' ');
+                for stream in array.iter::<Stream<'_>>() {
+                    let data = match stream.decoded() {
+                        Ok(data) => data,
+                        Err(DecodeFailure::TooLarge) => return TOO_LARGE,
+                        Err(_) => continue,
+                    };
+                    if collected.len() + data.len() >= MAX_DECODED_BYTES {
+                        return TOO_LARGE;
                     }
+                    crate::filter::reserve_within(
+                        &mut collected,
+                        data.len() + 1,
+                        MAX_DECODED_BYTES,
+                    );
+                    collected.extend_from_slice(&data);
+                    // Streams must have at least one whitespace in-between.
+                    collected.push(b' ');
+                }
 
-                    Some(collected)
-                } else {
-                    warn!("contents entry of page was neither stream nor array of streams");
+                (Some(collected), false)
+            } else {
+                warn!("contents entry of page was neither stream nor array of streams");
 
-                    None
-                }
-            })
-            .as_ref()
-            .map(|d| d.as_slice())
+                (None, false)
+            }
+        })
     }
 
     /// Get the resources of the page.
```
