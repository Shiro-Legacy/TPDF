// SPDX-License-Identifier: MIT
//! Image samples as the view decodes them (hayro's filters, image codecs
//! included: DCT, JPX, JBIG2, CCITT), for redaction to blank pixels in a
//! copy of an image the strict decoder cannot read.
use super::ViewDocument;
use hayro_syntax::object::stream::{ImageColorSpace, ImageDecodeParams};
use hayro_syntax::object::{ObjectIdentifier, Stream};

/// What the image's dictionary says, for the codecs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImageParams {
    pub indexed: bool,
    pub bits: Option<u8>,
    pub components: Option<u8>,
    pub width: u32,
    pub height: u32,
}

/// Decoded samples. `width`, `height`, `bits` and `components` are the
/// codec's when it reports them (a JPEG's own size), else None.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageSamples {
    pub data: Vec<u8>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bits: Option<u8>,
    pub components: Option<u8>,
    /// A JPX image's own alpha channel, `bits` per sample.
    pub alpha: Option<Vec<u8>>,
}

impl ViewDocument {
    /// The decoded samples of image stream `number` `generation`; None when
    /// it is not a stream or does not decode.
    pub fn decoded_image(
        &self,
        number: u32,
        generation: u16,
        params: ImageParams,
    ) -> Option<ImageSamples> {
        let id = ObjectIdentifier {
            obj_number: i32::try_from(number).ok()?,
            gen_number: i32::from(generation),
        };
        let stream = self.pdf.xref().get::<Stream<'_>>(id)?;
        let decoded = stream
            .decoded_image(&ImageDecodeParams {
                is_indexed: params.indexed,
                bpc: params.bits,
                num_components: params.components,
                target_dimension: None,
                width: params.width,
                height: params.height,
            })
            .ok()?;
        let info = decoded.image_data;
        Some(ImageSamples {
            data: decoded.data.into_owned(),
            width: info.as_ref().map(|i| i.width),
            height: info.as_ref().map(|i| i.height),
            bits: info.as_ref().map(|i| i.bits_per_component),
            components: info
                .as_ref()
                .and_then(|i| i.color_space)
                .map(|space| match space {
                    ImageColorSpace::Gray => 1,
                    ImageColorSpace::Rgb => 3,
                    ImageColorSpace::Cmyk => 4,
                    ImageColorSpace::Unknown(n) => n,
                }),
            alpha: info.and_then(|i| i.alpha),
        })
    }
}
