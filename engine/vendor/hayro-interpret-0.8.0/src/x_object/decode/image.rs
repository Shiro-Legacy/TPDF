use super::mask::decode_mask;
use super::{
    DecodeContext, REDUCED_CACHE_BYTES, box_reduce, decode_context, decode_u8_samples,
    fit_reduction, fix_image_length, over_budget, reduction, unpack_samples, unpacked_rows,
};
use crate::color::{ColorComponents, ToLuma, ToRgb};
use crate::interpret::state::ActiveTransferFunction;
use crate::util::hash128;
use crate::x_object::image::ImageXObject;
use crate::{ImageData, LumaData, RgbData};
use hayro_syntax::object::dict::keys::*;
use hayro_syntax::object::{Object, Stream};
use smallvec::SmallVec;

#[derive(Clone)]
pub(crate) struct DecodedImage {
    pub(crate) image: ImageData,
    pub(crate) alpha: Option<LumaData>,
}

pub(crate) fn decode_image(
    obj: &ImageXObject<'_>,
    target_dimension: Option<(u32, u32)>,
) -> Option<DecodedImage> {
    // Tandem patch: an image drawn much smaller than its samples is reduced
    // by whole-number box factors, and a decoded image is kept for the
    // render (a page may draw one image many times) when it decodes alike
    // wherever it is drawn.
    let decode = || {
        let mut decoded = ImageDecoder::new(obj, target_dimension)?.decode()?;
        // A decoder that already reduced the samples (JPX skips resolution
        // levels, an image past its budget is decoded reduced) leaves less.
        let (width, height) = (decoded.image.width(), decoded.image.height());
        let (kx, ky) = reduction(width, height, target_dimension);
        if kx > 1 || ky > 1 {
            decoded.image = reduce_image(decoded.image, kx, ky);
        }
        Some(decoded)
    };
    if !decodes_alike(obj) {
        return decode();
    }
    let dict = obj.stream.dict();
    let key = hash128(&(
        b"image",
        reduction(obj.width, obj.height, target_dimension),
        // JPX decoding picks a resolution from the target.
        target_dimension.map(|(w, h)| (w.next_power_of_two(), h.next_power_of_two())),
        dict.data(),
        obj.stream.raw_data().as_ref(),
    ));
    let size = |decoded: &DecodedImage| {
        let image = match &decoded.image {
            ImageData::Rgb(rgb) => rgb.data.len(),
            ImageData::Luma(luma) => luma.data.len(),
        };
        image + decoded.alpha.as_ref().map_or(0, |a| a.data.len())
    };
    obj.cache
        .get_or_insert_sized(key, REDUCED_CACHE_BYTES, size, decode)
}

/// Tandem patch: whether `obj` decodes alike wherever it is drawn: no
/// transfer function, no colour space named in the resources.
pub(crate) fn decodes_alike(obj: &ImageXObject<'_>) -> bool {
    let dict = obj.stream.dict();
    let named_space = dict
        .get::<Object<'_>>(CS)
        .or_else(|| dict.get::<Object<'_>>(COLORSPACE))
        .is_some_and(|cs| match cs {
            Object::Name(name) => !matches!(
                name.as_ref(),
                DEVICE_RGB | RGB | DEVICE_GRAY | G | DEVICE_CMYK | CMYK | CALCMYK
            ),
            _ => false,
        });
    obj.transfer_function.is_none() && !named_space
}

/// Tandem patch: `image` averaged over `kx` × `ky` blocks (unchanged when
/// its data does not hold its size).
fn reduce_image(image: ImageData, kx: usize, ky: usize) -> ImageData {
    let reduce = |data: &[u8], width: u32, height: u32, n: usize| {
        let (w, h) = (width as usize, height as usize);
        let (kx, ky) = (kx.clamp(1, w.max(1)), ky.clamp(1, h.max(1)));
        Some((
            box_reduce(data, w, h, n, kx, ky)?,
            w.div_ceil(kx) as u32,
            h.div_ceil(ky) as u32,
        ))
    };
    let scale = |(sx, sy): (f32, f32), width: u32, height: u32, w: u32, h: u32| {
        (
            sx * width as f32 / w.max(1) as f32,
            sy * height as f32 / h.max(1) as f32,
        )
    };
    match image {
        ImageData::Rgb(rgb) => {
            let Some((data, width, height)) = reduce(&rgb.data, rgb.width, rgb.height, 3) else {
                return ImageData::Rgb(rgb);
            };
            ImageData::Rgb(RgbData {
                data,
                scale_factors: scale(rgb.scale_factors, rgb.width, rgb.height, width, height),
                width,
                height,
                interpolate: rgb.interpolate,
            })
        }
        ImageData::Luma(luma) => {
            let Some((data, width, height)) = reduce(&luma.data, luma.width, luma.height, 1)
            else {
                return ImageData::Luma(luma);
            };
            ImageData::Luma(LumaData {
                data,
                scale_factors: scale(luma.scale_factors, luma.width, luma.height, width, height),
                width,
                height,
                interpolate: luma.interpolate,
            })
        }
    }
}

struct ImageDecoder<'a, 'b> {
    obj: &'a ImageXObject<'b>,
    ctx: DecodeContext<'b>,
    target_dimension: Option<(u32, u32)>,
    color_key_mask: Option<SmallVec<[u16; 4]>>,
    decoded_color_key_mask: Option<LumaData>,
}

/// Tandem patch: samples a row of an image decoded reduced may have. A row
/// is decoded, converted and summed whole, so its buffers (the row, its
/// components, an Indexed image's RGB) grow with its width, whatever it is
/// drawn at: an image 2^25 samples wide held 208 MB for a few rows. 2^20
/// keeps them within about 8 MB; real images are at most a few hundred
/// thousand samples wide.
const MAX_REDUCED_ROW_SAMPLES: u64 = 1 << 20;

/// Tandem patch: an image being decoded reduced a row at a time: each row
/// is added into running sums of its `kx`-wide column blocks, and every `ky`
/// rows (or at the end) the sums become a row of the reduced image, so what
/// is held is a row and the sums whatever the reduction.
struct Reduced {
    /// Components a sample.
    n: usize,
    kx: usize,
    ky: usize,
    row_bytes: u64,
    /// Rows of data (at most the image's height).
    rows: u64,
    out_width: u64,
    /// Whether a transfer function, if any, is a single one.
    single: bool,
    /// For 1, 2, 4, 8 and 16 bits: each component's value for each code,
    /// as the decode array (or an Indexed space) maps it.
    lut: Option<Vec<Vec<u8>>>,
    /// Whether blocks are averaged in the image's own colour space and
    /// converted once reduced (as reference viewers subsample before converting); an
    /// Indexed image is converted first (its indices do not average).
    source: bool,
    /// A row's components, reused from row to row.
    components: Vec<u8>,
    /// Whether the image reduces to luma, once the first row tells.
    luma: Option<bool>,
    /// Sums of the current block row: `out_width` × channels, and the
    /// colour key mask's `out_width`.
    sums: Vec<u64>,
    key_sums: Vec<u64>,
    /// Rows added to the current block row.
    block_rows: usize,
    out: Vec<u8>,
    key_out: Vec<u8>,
    /// The most bytes a row's own buffers took.
    row_buffers: usize,
}

impl<'a, 'b> ImageDecoder<'a, 'b> {
    fn new(obj: &'a ImageXObject<'b>, target_dimension: Option<(u32, u32)>) -> Option<Self> {
        let ctx = decode_context(obj, target_dimension, true)?;
        let color_key_mask = obj.stream.dict().get::<SmallVec<[u16; 4]>>(MASK);

        Some(Self {
            obj,
            ctx,
            target_dimension,
            color_key_mask,
            decoded_color_key_mask: None,
        })
    }

    fn decode(mut self) -> Option<DecodedImage> {
        let mut image = self.decode_image()?;
        let alpha = self.decode_alpha(&mut image);

        Some(DecodedImage { image, alpha })
    }

    fn decode_image(&mut self) -> Option<ImageData> {
        let num_components = self.ctx.color_space.num_components() as usize;

        // To prevent a panic when calling the `chunks` method.
        if num_components == 0 {
            return None;
        }
        // Tandem patch: data too large to hold is decoded as it inflates.
        if self.ctx.streamed {
            return self.decode_image_streamed(num_components);
        }
        // Tandem patch: samples within the image's budget are unpacked into
        // `rows`; past it they are decoded reduced, a strip at a time.
        let Some(rows) = unpacked_rows(
            self.ctx.decoded.data.len(),
            self.ctx.width,
            self.ctx.height,
            num_components,
            self.ctx.bits_per_component,
            self.obj.max_samples,
        ) else {
            return self.decode_image_reduced(num_components);
        };

        let component_ranges = self.ctx.color_space.component_ranges();
        let default_decode = self
            .ctx
            .color_space
            .default_decode_arr(self.ctx.bits_per_component as f32);
        let inverted_default_decode = self
            .ctx
            .color_space
            .inverted_default_decode_arr(self.ctx.bits_per_component as f32);
        let inverted_component_ranges = component_ranges
            .iter()
            .map(|(min, max)| (*max, *min))
            .collect::<SmallVec<[(f32, f32); 4]>>();
        let is_indexed = self.ctx.color_space.is_indexed();

        let direct_invert = if self.ctx.bits_per_component == 8
            && (self.ctx.decode_arr == component_ranges
                || is_indexed && self.ctx.decode_arr == default_decode)
        {
            Some(false)
        } else if self.ctx.bits_per_component == 8
            && (self.ctx.decode_arr == inverted_component_ranges
                || is_indexed && self.ctx.decode_arr == inverted_default_decode)
        {
            Some(true)
        } else {
            None
        };

        let mut components = if let Some(invert) = direct_invert {
            // This is actually the most common case, where the PDF is embedded
            // in such a way where we don't need to decode. In this case,
            // we can use the raw decoded component values directly.
            fix_image_length(
                self.ctx.decoded.data.to_mut(),
                self.ctx.width,
                &mut self.ctx.height,
                0,
                num_components,
            )?;

            if self.color_key_mask.is_some() {
                self.decoded_color_key_mask = self.decode_color_key_mask();
            }

            if invert {
                for value in self.ctx.decoded.data.to_mut() {
                    *value = 255 - *value;
                }
            }

            core::mem::take(&mut self.ctx.decoded.data).into_owned()
        } else {
            let mut components = decode_u8_samples(
                &self.ctx.decoded.data,
                self.ctx.width,
                rows,
                &self.ctx.color_space,
                self.ctx.bits_per_component,
                &self.ctx.decode_arr,
            )?;

            fix_image_length(
                &mut components,
                self.ctx.width,
                &mut self.ctx.height,
                0,
                num_components,
            )?;

            components
        };

        if self
            .obj
            .transfer_function
            .as_ref()
            .is_none_or(|t| matches!(t, ActiveTransferFunction::Single(_)))
            && self.ctx.color_space.to_luma(&mut components).is_some()
        {
            if let Some(transfer_function) = &self.obj.transfer_function {
                transfer_function.apply_to(&mut components);
            }

            return Some(ImageData::Luma(LumaData {
                data: components,
                width: self.ctx.width,
                height: self.ctx.height,
                interpolate: self.obj.interpolate,
                scale_factors: self.ctx.scale_factors,
            }));
        }

        let mut rgb_data = self.convert_to_rgb(components)?;

        self.apply_transfer_function(&mut rgb_data);

        Some(ImageData::Rgb(rgb_data))
    }

    /// Tandem patch: an image whose samples pass its budget, decoded
    /// reduced toward the size it is drawn a row at a time, so its samples
    /// are never held whole; `None` when it is not drawn smaller or still
    /// passes the budget reduced.
    fn decode_image_reduced(&mut self, n: usize) -> Option<ImageData> {
        let data = &self.ctx.decoded.data;
        let mut reduced = self.reduced(n, data.len() as u64)?;
        let held = usize::try_from(reduced.row_bytes * reduced.rows)
            .map_or(data.len(), |n| n.min(data.len()));
        let row_bytes = reduced.row_bytes as usize;
        for row in data[..held].chunks(row_bytes) {
            self.reduce_row(&mut reduced, row)?;
        }
        self.finish_reduced(reduced)
    }

    /// Tandem patch: an image whose data would decode past what hayro-syntax
    /// holds for an image (`MAX_IMAGE_DECODED_BYTES`), decoded reduced as
    /// [`Self::decode_image_reduced`] while its data is inflated, a row at
    /// a time (reference viewers subsample such an image as they decode; hayro
    /// held its data whole, and a 24000 × 24000 grayscale image grew WASM
    /// memory to about 1 GB before it was refused). Its rows past the
    /// declared height are not decoded.
    fn decode_image_streamed(&mut self, n: usize) -> Option<ImageData> {
        let row_bytes = Self::row_bytes(self.ctx.width, n, self.ctx.bits_per_component);
        let mut reduced = self.reduced(n, row_bytes.saturating_mul(u64::from(self.ctx.height)))?;
        let row_len = reduced.row_bytes as usize;
        let total = reduced.row_bytes * reduced.rows;
        let mut row = Vec::with_capacity(row_len);
        let (mut taken, mut failed) = (0u64, false);
        let decoded = self.obj.stream.decoded_streamed(&mut |mut chunk: &[u8]| {
            while !chunk.is_empty() && taken < total {
                let room = (row_len - row.len()) as u64;
                let take = room.min(total - taken).min(chunk.len() as u64) as usize;
                row.extend_from_slice(&chunk[..take]);
                chunk = &chunk[take..];
                taken += take as u64;
                if row.len() == row_len {
                    if self.reduce_row(&mut reduced, &row).is_none() {
                        failed = true;
                        return false;
                    }
                    row.clear();
                }
            }
            taken < total
        });
        if failed || decoded != Some(true) || taken == 0 {
            return None;
        }
        if !row.is_empty() {
            self.reduce_row(&mut reduced, &row)?;
        }
        // The row assembled here, and what the inflater holds: its 64 KiB
        // chunk and, with a predictor, the previous and current rows.
        reduced.row_buffers += row.capacity() + 2 * row_len + (1 << 16);
        reduced.rows = taken.div_ceil(reduced.row_bytes);
        self.finish_reduced(reduced)
    }

    /// Bytes of one row of `width` samples of `n` components.
    fn row_bytes(width: u32, n: usize, bits: u8) -> u64 {
        (u64::from(width) * n as u64 * u64::from(bits))
            .div_ceil(8)
            .max(1)
    }

    /// The reduction of an image of `n` components whose data holds `len`
    /// bytes: `None` (over budget) when it is not drawn smaller, still
    /// passes the budget reduced, or its samples pass what the render may
    /// decode reduced (`Cache::reserve_reduced_samples`).
    fn reduced(&self, n: usize, len: u64) -> Option<Reduced> {
        let (width, bits) = (self.ctx.width, self.ctx.bits_per_component);
        let (kx, ky) = fit_reduction(width, self.ctx.height, self.target_dimension);
        let (kx, ky) = (kx.clamp(1, width.max(1) as usize), ky.max(1));
        let row_bytes = Self::row_bytes(width, n, bits);
        let rows = len.div_ceil(row_bytes).min(u64::from(self.ctx.height));
        let (out_width, out_height) = (
            u64::from(width).div_ceil(kx as u64),
            rows.div_ceil(ky as u64),
        );
        if !((kx > 1 || ky > 1) && out_width * out_height * n as u64 <= self.obj.max_samples)
            || u64::from(width).saturating_mul(n as u64) > MAX_REDUCED_ROW_SAMPLES
            || usize::try_from(row_bytes).is_err()
            || !self
                .obj
                .cache
                .reserve_reduced_samples(u64::from(width).saturating_mul(rows).saturating_mul(n as u64))
        {
            return over_budget(self.obj);
        }
        let single = self
            .obj
            .transfer_function
            .as_ref()
            .is_none_or(|t| matches!(t, ActiveTransferFunction::Single(_)));
        Some(Reduced {
            n,
            kx,
            ky,
            row_bytes,
            rows,
            out_width,
            single,
            lut: self.code_table(n),
            source: !self.ctx.color_space.is_indexed(),
            components: Vec::new(),
            luma: None,
            sums: Vec::new(),
            key_sums: Vec::new(),
            block_rows: 0,
            out: Vec::new(),
            key_out: Vec::new(),
            row_buffers: 0,
        })
    }

    /// For 1, 2, 4, 8 and 16 bits, each component's decoded value for each
    /// code, read through the image's own decode (one row holding every
    /// code), so a row is decoded by table instead of per sample.
    fn code_table(&self, n: usize) -> Option<Vec<Vec<u8>>> {
        let bits = self.ctx.bits_per_component;
        if !matches!(bits, 1 | 2 | 4 | 8 | 16) || n == 0 || n > 32 {
            return None;
        }
        let codes = 1usize << bits;
        let mut packed = Vec::new();
        let (mut acc, mut filled) = (0u32, 0u8);
        for code in 0..codes {
            for _ in 0..n {
                if bits == 16 {
                    packed.extend_from_slice(&(code as u16).to_be_bytes());
                    continue;
                }
                acc = (acc << bits) | code as u32;
                filled += bits;
                if filled == 8 {
                    packed.push(acc as u8);
                    (acc, filled) = (0, 0);
                }
            }
        }
        if filled > 0 {
            packed.push((acc << (8 - filled)) as u8);
        }
        let decoded = decode_u8_samples(
            &packed,
            codes as u32,
            1,
            &self.ctx.color_space,
            bits,
            &self.ctx.decode_arr,
        )?;
        let mut table = vec![vec![0u8; codes]; n];
        for (index, value) in decoded.iter().enumerate().take(codes * n) {
            table[index % n][index / n] = *value;
        }
        Some(table)
    }

    /// Adds one row of data (the last may be short; what it lacks reads as
    /// 0) into `reduced`, and ends a block row every `ky` rows.
    fn reduce_row(&self, reduced: &mut Reduced, row: &[u8]) -> Option<()> {
        let (width, bits, n) = (self.ctx.width, self.ctx.bits_per_component, reduced.n);
        let row_len = reduced.row_bytes as usize;
        let samples = width as usize * n;
        let padded;
        let (row, present) = if row.len() < row_len {
            let mut whole = row.to_vec();
            whole.resize(row_len, 0);
            padded = whole;
            (&padded[..], (row.len() * 8 / usize::from(bits)).min(samples))
        } else {
            (row, samples)
        };
        let mut buffers = padded_len(row_len, row.len());
        if let Some(key) = &self.color_key_mask {
            let mut components = unpack_samples(row, width, 1, n, bits)?;
            components.resize(samples, 0);
            components[present..].fill(0);
            let keyed = key_mask(&components, n, key);
            buffers += components.capacity() * 2 + keyed.capacity();
            add_columns(&mut reduced.key_sums, &keyed, 1, reduced.kx, reduced.out_width);
        }
        let mut components = std::mem::take(&mut reduced.components);
        match &reduced.lut {
            Some(table) => decode_by_table(row, samples, n, bits, table, &mut components),
            None => {
                components = decode_u8_samples(
                    row,
                    width,
                    1,
                    &self.ctx.color_space,
                    bits,
                    &self.ctx.decode_arr,
                )?
            }
        }
        components.resize(samples, 0);
        components[present..].fill(0);
        if reduced.source {
            add_columns(&mut reduced.sums, &components, n, reduced.kx, reduced.out_width);
            buffers += components.capacity();
            reduced.components = components;
        } else {
            // The row's components and their conversion, at once.
            buffers += components.capacity();
            let pixels = self.convert_reduced(reduced, components)?;
            if pixels.len() % width.max(1) as usize != 0 {
                return None;
            }
            let channels = pixels.len() / width.max(1) as usize;
            buffers += pixels.capacity();
            add_columns(&mut reduced.sums, &pixels, channels, reduced.kx, reduced.out_width);
        }
        reduced.row_buffers = reduced.row_buffers.max(buffers);
        reduced.block_rows += 1;
        if reduced.block_rows == reduced.ky {
            self.end_block_row(reduced)?;
        }
        Some(())
    }

    /// `components` (a row, or a reduced row) as luma or RGB, with the
    /// transfer function; whether the image is luma is settled by the first.
    fn convert_reduced(&self, reduced: &mut Reduced, mut components: Vec<u8>) -> Option<Vec<u8>> {
        let is_luma = match reduced.luma {
            None => *reduced
                .luma
                .insert(reduced.single && self.ctx.color_space.to_luma(&mut components).is_some()),
            Some(true) => {
                self.ctx.color_space.to_luma(&mut components)?;
                true
            }
            Some(false) => false,
        };
        let mut pixels = if is_luma {
            components
        } else {
            let pixels = components.len() / reduced.n.max(1);
            if self
                .ctx
                .color_space
                .convert_in_place(&mut components)
                .is_some()
            {
                components
            } else {
                let mut output = vec![0; pixels * 3];
                self.ctx.color_space.convert(&components, &mut output)?;
                output
            }
        };
        if let Some(transfer_function) = &self.obj.transfer_function {
            transfer_function.apply_to(&mut pixels);
        }
        Some(pixels)
    }

    /// Turns the sums of the current block row into a row of the reduced
    /// image (and of its colour key mask), each sample the rounded mean of
    /// its block (partial blocks at the edges average what they hold),
    /// converted now when blocks are averaged in the image's own space.
    fn end_block_row(&self, reduced: &mut Reduced) -> Option<()> {
        let rows = reduced.block_rows as u64;
        if rows == 0 {
            return Some(());
        }
        let (width, kx) = (u64::from(self.ctx.width), reduced.kx as u64);
        let mean = |sums: &mut Vec<u64>, out: &mut Vec<u8>, channels: usize| {
            for (bx, block) in sums.chunks_exact_mut(channels.max(1)).enumerate() {
                let bx = bx as u64;
                let count = ((bx * kx + kx).min(width) - bx * kx) * rows;
                for sum in block.iter_mut() {
                    out.push(((*sum + count / 2) / count.max(1)) as u8);
                    *sum = 0;
                }
            }
        };
        let channels = reduced.sums.len() / reduced.out_width.max(1) as usize;
        if reduced.source {
            let mut row = Vec::with_capacity(reduced.sums.len());
            mean(&mut reduced.sums, &mut row, channels);
            let pixels = self.convert_reduced(reduced, row)?;
            reduced.out.extend_from_slice(&pixels);
        } else {
            mean(&mut reduced.sums, &mut reduced.out, channels);
        }
        mean(&mut reduced.key_sums, &mut reduced.key_out, 1);
        reduced.block_rows = 0;
        Some(())
    }

    /// The image `reduced` holds, and its colour key mask.
    fn finish_reduced(&mut self, mut reduced: Reduced) -> Option<ImageData> {
        self.end_block_row(&mut reduced)?;
        let working = reduced.row_buffers
            + (reduced.sums.capacity() + reduced.key_sums.capacity()) * 8
            + reduced.out.capacity()
            + reduced.key_out.capacity()
            + reduced.lut.as_ref().map_or(0, |t| t.iter().map(Vec::len).sum());
        self.obj.cache.note_reduced_working_set(working);
        let rows = reduced.rows;
        let out_height = rows.div_ceil(reduced.ky as u64);
        let out_width = reduced.out_width;
        if reduced.out.is_empty() {
            return None;
        }
        let (sx, sy) = self.ctx.scale_factors;
        let scale_factors = (
            sx * self.ctx.width as f32 / out_width.max(1) as f32,
            sy * rows as f32 / out_height.max(1) as f32,
        );
        let (width, height) = (out_width as u32, out_height as u32);
        let interpolate = self.obj.interpolate;
        if self.color_key_mask.is_some() {
            self.decoded_color_key_mask = Some(LumaData {
                data: reduced.key_out,
                width,
                height,
                interpolate,
                scale_factors,
            });
        }
        Some(if reduced.luma == Some(true) {
            ImageData::Luma(LumaData {
                data: reduced.out,
                width,
                height,
                interpolate,
                scale_factors,
            })
        } else {
            ImageData::Rgb(RgbData {
                data: reduced.out,
                width,
                height,
                interpolate,
                scale_factors,
            })
        })
    }

    /// `rows` rows of decoded components as RGB.
    fn convert_rows(&self, mut decoded: Vec<u8>, rows: u32) -> Option<Vec<u8>> {
        if self
            .ctx
            .color_space
            .convert_in_place(&mut decoded)
            .is_none()
        {
            let mut output = vec![0; self.ctx.width as usize * rows as usize * 3];
            self.ctx.color_space.convert(&decoded, &mut output)?;
            decoded = output;
        }
        Some(decoded)
    }

    fn convert_to_rgb(&self, decoded: Vec<u8>) -> Option<RgbData> {
        let decoded = self.convert_rows(decoded, self.ctx.height)?;

        Some(RgbData {
            data: decoded,
            width: self.ctx.width,
            height: self.ctx.height,
            interpolate: self.obj.interpolate,
            scale_factors: self.ctx.scale_factors,
        })
    }

    fn apply_transfer_function(&self, rgb_data: &mut RgbData) {
        if let Some(transfer_function) = &self.obj.transfer_function {
            transfer_function.apply_to(&mut rgb_data.data);
        }
    }

    fn decode_alpha(&mut self, image: &mut ImageData) -> Option<LumaData> {
        // Tandem patch: and the image's own size (an image past its budget
        // is decoded reduced).
        if let Some((alpha, matte_rgb)) = self.resolve_matte()
            && alpha.width == self.ctx.width
            && alpha.height == self.ctx.height
            && (image.width(), image.height()) == (alpha.width, alpha.height)
        {
            unpremultiply(image, &alpha.data, &matte_rgb);

            return Some(alpha);
        }

        // If the alpha channel is invalid, return no alpha so the main image can
        // still be returned (see PDFJS-19611).
        let dict = self.obj.stream.dict();

        if let Some(1) = dict.get::<u8>(SMASK_IN_DATA) {
            let mut data = self
                .ctx
                .decoded
                .image_data
                .as_mut()
                .and_then(|image| image.alpha.take())?;
            fix_image_length(&mut data, self.ctx.width, &mut self.ctx.height, 0, 1)?;

            Some(LumaData {
                data,
                width: self.ctx.width,
                height: self.ctx.height,
                interpolate: self.obj.interpolate,
                scale_factors: self.ctx.scale_factors,
            })
            // Note: `SMASK` field takes precedence over `MASK`, so order matters here.
        } else if let Some(s_mask) = dict
            .get::<Stream<'_>>(SMASK)
            .or_else(|| dict.get::<Stream<'_>>(MASK))
        {
            let obj = ImageXObject::new_mask(&s_mask, self.obj)?;

            decode_mask(&obj, self.target_dimension).map(|decoded| decoded.luma)
        } else if self.color_key_mask.is_some() {
            self.decoded_color_key_mask
                .take()
                .or_else(|| self.decode_color_key_mask())
        } else {
            None
        }
    }

    fn resolve_matte(&self) -> Option<(LumaData, [u8; 3])> {
        let s_mask = self.obj.stream.dict().get::<Stream<'_>>(SMASK)?;
        let matte = s_mask.dict().get::<ColorComponents>(MATTE)?;

        if matte.len() != self.ctx.color_space.num_components() as usize {
            return None;
        }

        // In theory, matte needs to be applied in the image's original color space,
        // but we always do it in RGB for now.
        let mut matte_rgb = [0_u8; 3];
        self.ctx.color_space.convert_values(&matte, &mut matte_rgb);

        let mask_obj = ImageXObject::new_mask(&s_mask, self.obj)?;
        let alpha = decode_mask(&mask_obj, self.target_dimension)?.luma;

        Some((alpha, matte_rgb))
    }

    fn decode_color_key_mask(&self) -> Option<LumaData> {
        let color_key_mask = self.color_key_mask.as_deref()?;
        let num_components = self.ctx.color_space.num_components() as usize;
        // Tandem patch: unpacked within the image's budget.
        let Some(rows) = unpacked_rows(
            self.ctx.decoded.data.len(),
            self.ctx.width,
            self.ctx.height,
            num_components,
            self.ctx.bits_per_component,
            self.obj.max_samples,
        ) else {
            return over_budget(self.obj);
        };
        let components = unpack_samples(
            &self.ctx.decoded.data,
            self.ctx.width,
            rows,
            num_components,
            self.ctx.bits_per_component,
        )?;
        let mut data = key_mask(&components, num_components, color_key_mask);

        let mut height = self.ctx.height;
        fix_image_length(&mut data, self.ctx.width, &mut height, 0, 1)?;

        Some(LumaData {
            data,
            width: self.ctx.width,
            height,
            interpolate: self.obj.interpolate,
            scale_factors: self.ctx.scale_factors,
        })
    }
}

/// The colour key mask of `components` (`n` a pixel): 0 where every
/// component is within its range of `key`, 255 elsewhere.
fn key_mask(components: &[u16], n: usize, key: &[u16]) -> Vec<u8> {
    let mut data = Vec::with_capacity(components.len() / n.max(1));

    for pixel in components.chunks_exact(n) {
        let mut value = 0;

        for (component, min_max) in pixel.iter().zip(key.chunks_exact(2)) {
            if *component > min_max[1] || *component < min_max[0] {
                value = 255;
            }
        }

        data.push(value);
    }

    data
}

fn unpremultiply(image: &mut ImageData, alpha: &[u8], matte_rgb: &[u8]) {
    match image {
        ImageData::Rgb(rgb) => {
            for (pixel, &a) in rgb.data.chunks_exact_mut(3).zip(alpha.iter()) {
                if a == 0 {
                    continue;
                }
                let inv_alpha = 255.0 / a as f32;
                for (c, &m) in pixel.iter_mut().zip(matte_rgb.iter()) {
                    let m = m as f32;
                    *c = (m + (*c as f32 - m) * inv_alpha) as u8;
                }
            }
        }
        ImageData::Luma(luma) => {
            let m = matte_rgb[0] as f32;
            for (c, &a) in luma.data.iter_mut().zip(alpha.iter()) {
                if a == 0 {
                    continue;
                }
                let inv_alpha = 255.0 / a as f32;
                *c = (m + (*c as f32 - m) * inv_alpha) as u8;
            }
        }
    }
}

/// Tandem patch: the bytes a short row was padded into.
fn padded_len(row_len: usize, len: usize) -> usize {
    if len < row_len { row_len } else { 0 }
}

/// Tandem patch: adds a row of `pixels` (`channels` a pixel) into `sums`,
/// one entry per channel of each `kx`-wide column block (`out_width` blocks).
fn add_columns(sums: &mut Vec<u64>, pixels: &[u8], channels: usize, kx: usize, out_width: u64) {
    let len = out_width as usize * channels;
    if sums.len() != len {
        sums.clear();
        sums.resize(len, 0);
    }
    // A block of fewer than 2^24 samples sums within a u32.
    if channels == 1 && kx < 1 << 24 {
        for (block, sum) in pixels.chunks(kx).zip(sums.iter_mut()) {
            *sum += block.iter().map(|&v| u32::from(v)).sum::<u32>() as u64;
        }
        return;
    }
    for (block, sum) in pixels
        .chunks(kx * channels)
        .zip(sums.chunks_exact_mut(channels))
    {
        for pixel in block.chunks_exact(channels) {
            for (s, v) in sum.iter_mut().zip(pixel) {
                *s += u64::from(*v);
            }
        }
    }
}

/// Tandem patch: `samples` samples of a row of 1, 2, 4, 8 or 16 bits into
/// `out`, each component's code mapped by `table`.
fn decode_by_table(
    row: &[u8],
    samples: usize,
    n: usize,
    bits: u8,
    table: &[Vec<u8>],
    out: &mut Vec<u8>,
) {
    out.clear();
    out.reserve(samples);
    let mut component = 0;
    let mut push = |code: usize, out: &mut Vec<u8>| {
        out.push(table[component][code]);
        component += 1;
        if component == n {
            component = 0;
        }
    };
    if n == 1 && bits == 8 {
        let table = &table[0];
        out.extend(row.iter().take(samples).map(|&byte| table[usize::from(byte)]));
        return;
    }
    match bits {
        16 => {
            for pair in row.chunks_exact(2).take(samples) {
                push(usize::from(u16::from_be_bytes([pair[0], pair[1]])), out);
            }
        }
        8 => {
            for &byte in row.iter().take(samples) {
                push(usize::from(byte), out);
            }
        }
        _ => {
            let per_byte = 8 / bits;
            let mask = (1u16 << bits) - 1;
            'bytes: for &byte in row {
                for k in 0..per_byte {
                    if out.len() == samples {
                        break 'bytes;
                    }
                    push(
                        usize::from((u16::from(byte) >> (8 - bits * (k + 1))) & mask),
                        out,
                    );
                }
            }
        }
    }
}
