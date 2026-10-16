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

impl<'a, 'b> ImageDecoder<'a, 'b> {
    fn new(obj: &'a ImageXObject<'b>, target_dimension: Option<(u32, u32)>) -> Option<Self> {
        let ctx = decode_context(obj, target_dimension)?;
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
    /// reduced toward the size it is drawn at a strip of rows at a time, so
    /// its samples are never held whole; `None` when it is not drawn
    /// smaller or still passes the budget reduced.
    fn decode_image_reduced(&mut self, n: usize) -> Option<ImageData> {
        let (width, bits) = (self.ctx.width, self.ctx.bits_per_component);
        let (kx, ky) = fit_reduction(width, self.ctx.height, self.target_dimension);
        let row_bytes = (u64::from(width) * n as u64 * u64::from(bits))
            .div_ceil(8)
            .max(1);
        let data = &self.ctx.decoded.data;
        let rows = (data.len() as u64)
            .div_ceil(row_bytes)
            .min(u64::from(self.ctx.height));
        let (out_width, out_height) = (
            u64::from(width).div_ceil(kx as u64),
            rows.div_ceil(ky as u64),
        );
        let strip = usize::try_from(row_bytes * ky as u64).ok();
        let Some(strip) = strip.filter(|_| {
            (kx > 1 || ky > 1) && out_width * out_height * n as u64 <= self.obj.max_samples
        }) else {
            return over_budget(self.obj);
        };
        let single = self
            .obj
            .transfer_function
            .as_ref()
            .is_none_or(|t| matches!(t, ActiveTransferFunction::Single(_)));
        let key = self.color_key_mask.clone();
        let (mut luma, mut out, mut key_out) = (None, Vec::new(), Vec::new());
        let held = usize::try_from(row_bytes * rows).map_or(data.len(), |n| n.min(data.len()));
        for strip in data[..held].chunks(strip) {
            let strip_rows = (strip.len() as u64).div_ceil(row_bytes) as u32;
            let samples = width as usize * strip_rows as usize * n;
            if let Some(key) = &key {
                let mut components = unpack_samples(strip, width, strip_rows, n, bits)?;
                components.resize(samples, 0);
                let keyed = key_mask(&components, n, key);
                key_out.extend(box_reduce(&keyed, width as usize, strip_rows as usize, 1, kx, ky)?);
            }
            let mut components = decode_u8_samples(
                strip,
                width,
                strip_rows,
                &self.ctx.color_space,
                bits,
                &self.ctx.decode_arr,
            )?;
            components.resize(samples, 0);
            let is_luma = match luma {
                None => *luma.insert(single && self.ctx.color_space.to_luma(&mut components).is_some()),
                Some(true) => {
                    self.ctx.color_space.to_luma(&mut components)?;
                    true
                }
                Some(false) => false,
            };
            let (mut pixels, channels) = if is_luma {
                (components, 1)
            } else {
                (self.convert_rows(components, strip_rows)?, 3)
            };
            if let Some(transfer_function) = &self.obj.transfer_function {
                transfer_function.apply_to(&mut pixels);
            }
            out.extend(box_reduce(
                &pixels,
                width as usize,
                strip_rows as usize,
                channels,
                kx,
                ky,
            )?);
        }
        let (sx, sy) = self.ctx.scale_factors;
        let scale_factors = (
            sx * width as f32 / out_width.max(1) as f32,
            sy * rows as f32 / out_height.max(1) as f32,
        );
        let (width, height) = (out_width as u32, out_height as u32);
        let interpolate = self.obj.interpolate;
        if key.is_some() {
            self.decoded_color_key_mask = Some(LumaData {
                data: key_out,
                width,
                height,
                interpolate,
                scale_factors,
            });
        }
        Some(if luma == Some(true) {
            ImageData::Luma(LumaData {
                data: out,
                width,
                height,
                interpolate,
                scale_factors,
            })
        } else {
            ImageData::Rgb(RgbData {
                data: out,
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
