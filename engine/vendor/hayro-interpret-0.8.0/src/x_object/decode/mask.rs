use super::{
    DecodeContext, REDUCED_CACHE_BYTES, box_reduce, decode_context, fix_image_length, reduction,
    unpack_samples,
};
use crate::LumaData;
use crate::function::interpolate;
use crate::util::hash128;
use crate::x_object::image::{ImageKind, ImageXObject};
use std::sync::LazyLock;

static BILEVEL_MASK_LUT: LazyLock<Box<[u64; 256]>> = LazyLock::new(|| {
    let mut lut = Box::new([0; 256]);
    for (byte, expanded) in lut.iter_mut().enumerate() {
        let mut pixels = [0; 8];
        for (bit, pixel) in pixels.iter_mut().enumerate() {
            *pixel = if byte & (0x80 >> bit) != 0 { 255 } else { 0 };
        }
        *expanded = u64::from_ne_bytes(pixels);
    }
    lut
});

pub(crate) struct DecodedMask {
    pub(crate) luma: LumaData,
}

pub(crate) fn decode_mask(
    obj: &ImageXObject<'_>,
    target_dimension: Option<(u32, u32)>,
) -> Option<DecodedMask> {
    // Tandem patch: a mask drawn much smaller than its samples is decoded
    // reduced by whole-number box factors, once per render.
    let (kx, ky) = reduction(obj.width, obj.height, target_dimension);
    if kx > 1 || ky > 1 {
        let invert = obj.kind == ImageKind::StencilMask;
        let key = hash128(&(
            b"mask",
            kx,
            ky,
            invert,
            obj.stream.dict().data(),
            obj.stream.raw_data().as_ref(),
        ));
        let luma = obj.cache.get_or_insert_sized(
            key,
            REDUCED_CACHE_BYTES,
            |luma: &LumaData| luma.data.len(),
            || {
                let ctx = decode_context(obj, target_dimension)?;
                // A decoder that already reduced the samples leaves less.
                let (kx, ky) = reduction(ctx.width, ctx.height, target_dimension);
                let scale_factors = ctx.scale_factors;
                let (data, width, height) = decode_mask_data_reduced(ctx, invert, kx, ky)?;
                Some(LumaData {
                    data,
                    width: width as u32,
                    height: height as u32,
                    interpolate: obj.interpolate,
                    scale_factors: (scale_factors.0 * kx as f32, scale_factors.1 * ky as f32),
                })
            },
        )?;
        return Some(DecodedMask { luma });
    }
    let ctx = decode_context(obj, target_dimension)?;
    let width = ctx.width;
    let scale_factors = ctx.scale_factors;

    // Note: The semantics between "normal" soft masks (i.e. masks defined in
    // the graphics state or via `Mask`/`SMask` are inverted compared to
    // stencil masks (defined via `ImageMask`). The former match the semantics
    // of normal alpha images, where 0 stands for invisible and MAX stands for
    // fully opaque. For stencil masks, it's the other way around: 1 means the
    // paint is visible, while 0 means it's invisible.
    let invert = obj.kind == ImageKind::StencilMask;
    let (data, height) = decode_mask_data(ctx, invert)?;

    Some(DecodedMask {
        luma: LumaData {
            data,
            width,
            height,
            interpolate: obj.interpolate,
            scale_factors,
        },
    })
}

fn decode_mask_data(mut ctx: DecodeContext<'_>, invert: bool) -> Option<(Vec<u8>, u32)> {
    let default_decode = ctx
        .color_space
        .default_decode_arr(ctx.bits_per_component as f32);
    let inverted_default = ctx
        .color_space
        .inverted_default_decode_arr(ctx.bits_per_component as f32);

    let bilevel_fast_path = ctx.bits_per_component == 1
        && ctx.color_space.num_components() == 1
        && (ctx.decode_arr.as_slice() == default_decode.as_slice()
            || ctx.decode_arr.as_slice() == inverted_default.as_slice());
    let fast_path = ctx.bits_per_component == 8
        && (ctx.decode_arr.as_slice() == default_decode.as_slice()
            || ctx.decode_arr.as_slice() == inverted_default.as_slice());

    let mut data = if bilevel_fast_path {
        decode_bilevel_mask_data(
            &ctx,
            invert ^ (ctx.decode_arr.as_slice() == inverted_default.as_slice()),
        )
    } else if fast_path {
        let mut decoded = ctx.decoded.data;
        let should_invert = invert ^ (ctx.decode_arr.as_slice() == inverted_default.as_slice());
        if should_invert {
            for byte in decoded.to_mut() {
                *byte = 255 - *byte;
            }
        }

        decoded.into_owned()
    } else {
        let num_components = ctx.color_space.num_components() as usize;
        let components = unpack_samples(
            &ctx.decoded.data,
            ctx.width,
            ctx.height,
            num_components,
            ctx.bits_per_component,
        )?;

        let source_max = 2.0_f32.powi(ctx.bits_per_component as i32) - 1.0;
        let mut decoded = Vec::with_capacity(components.len());

        for pixel in components.chunks(num_components) {
            for (component, (decode_min, decode_max)) in pixel.iter().zip(&ctx.decode_arr) {
                let value =
                    interpolate(*component as f32, 0.0, source_max, *decode_min, *decode_max);
                let value = if invert { 1.0 - value } else { value };
                decoded.push((value * 255.0 + 0.5) as u8);
            }
        }

        decoded
    };

    fix_image_length(
        &mut data,
        ctx.width,
        &mut ctx.height,
        0,
        ctx.color_space.num_components() as usize,
    )?;

    Some((data, ctx.height))
}

/// Tandem patch: `decode_mask_data` reduced by `kx` × `ky` box factors; a
/// one-bit mask is counted from its bits without expanding them.
fn decode_mask_data_reduced(
    ctx: DecodeContext<'_>,
    invert: bool,
    kx: usize,
    ky: usize,
) -> Option<(Vec<u8>, usize, usize)> {
    let width = ctx.width as usize;
    let inverted_default = ctx
        .color_space
        .inverted_default_decode_arr(ctx.bits_per_component as f32);
    let default_decode = ctx
        .color_space
        .default_decode_arr(ctx.bits_per_component as f32);
    let bilevel = ctx.bits_per_component == 1
        && ctx.color_space.num_components() == 1
        && (ctx.decode_arr.as_slice() == default_decode.as_slice()
            || ctx.decode_arr.as_slice() == inverted_default.as_slice());
    if !bilevel {
        let (data, height) = decode_mask_data(ctx, invert)?;
        let height = height as usize;
        let reduced = box_reduce(&data, width, height, 1, kx, ky);
        return Some((reduced, width.div_ceil(kx), height.div_ceil(ky)));
    }
    let invert = invert ^ (ctx.decode_arr.as_slice() == inverted_default.as_slice());
    let height = ctx.height as usize;
    let out_width = width.div_ceil(kx);
    let mut counts = vec![0usize; out_width];
    let mut out = Vec::with_capacity(out_width * height.div_ceil(ky));
    let mut rows = ctx.decoded.data.chunks(width.div_ceil(8)).take(height);
    loop {
        counts.iter_mut().for_each(|c| *c = 0);
        let mut row_count = 0;
        for row in rows.by_ref().take(ky) {
            row_count += 1;
            count_ones(row, width, kx, &mut counts);
        }
        if row_count == 0 {
            break;
        }
        for (bx, count) in counts.iter().enumerate() {
            let n = (((bx + 1) * kx).min(width) - bx * kx) * row_count;
            let set = if invert { n - count } else { *count };
            out.push(((set * 255 + n / 2) / n) as u8);
        }
    }
    let mut out_height = height.div_ceil(ky) as u32;
    fix_image_length(&mut out, out_width as u32, &mut out_height, 0, 1)?;
    Some((out, out_width, out_height as usize))
}

/// Adds the set bits of the first `width` bits of `row` (most significant
/// first) to `counts`, one count per `kx` bits.
fn count_ones(row: &[u8], width: usize, kx: usize, counts: &mut [usize]) {
    for (index, &byte) in row.iter().enumerate() {
        let start = index * 8;
        if start >= width {
            break;
        }
        let end = (start + 8).min(width);
        let byte = byte & !(0xFFu16 >> (end - start)) as u8;
        if byte == 0 {
            continue;
        }
        let (first, last) = (start / kx, (end - 1) / kx);
        if first == last {
            if let Some(count) = counts.get_mut(first) {
                *count += byte.count_ones() as usize;
            }
        } else {
            for bit in 0..end - start {
                if byte & (0x80 >> bit) != 0
                    && let Some(count) = counts.get_mut((start + bit) / kx)
                {
                    *count += 1;
                }
            }
        }
    }
}

fn decode_bilevel_mask_data(ctx: &DecodeContext<'_>, should_invert: bool) -> Vec<u8> {
    let width = ctx.width as usize;
    let height = ctx.height as usize;
    let row_bytes = width.div_ceil(8);
    let data = ctx.decoded.data.as_ref();

    let xor_mask = if should_invert { u64::MAX } else { 0 };
    let full_bytes = width / 8;
    let tail_bits = width % 8;
    let lut = &**BILEVEL_MASK_LUT;
    // Tandem patch: reserved for the rows the data holds, not the declared size.
    let mut out = Vec::with_capacity(
        width
            .saturating_mul(height)
            .min(data.len().saturating_mul(8)),
    );
    // To avoid repeatedly calling `extend`, which turns out to be very
    // expensive.
    let mut buffer = [[0; 8]; 32];

    for row in data.chunks(row_bytes).take(height) {
        for chunk in row[..row.len().min(full_bytes)].chunks(buffer.len()) {
            for (expanded, &byte) in buffer.iter_mut().zip(chunk) {
                *expanded = (lut[byte as usize] ^ xor_mask).to_ne_bytes();
            }
            out.extend_from_slice(buffer[..chunk.len()].as_flattened());
        }

        if tail_bits != 0 && row.len() > full_bytes {
            let expanded = (lut[row[full_bytes] as usize] ^ xor_mask).to_ne_bytes();
            out.extend_from_slice(&expanded[..tail_bits]);
        }
    }
    out
}
