// SPDX-License-Identifier: AGPL-3.0-or-later
//! Decode a loaded simple font's declared Unicode mapping, without fallback,
//! glyph selection, positioning, metrics application or reading-order inference.
use crate::font_resources::{LoadedSimpleFont, SimpleFontMapping};
use crate::text_decode::{
    DecodedText, TextDecodeError, TextDecodeLimits, decode_encoded_text, decode_mapped_text,
};
pub fn decode_declared_text<'input>(
    font: &LoadedSimpleFont,
    input: &'input [u8],
    limits: TextDecodeLimits,
) -> Result<DecodedText<'input>, TextDecodeError> {
    match font.mapping() {
        SimpleFontMapping::ToUnicode(map) => decode_mapped_text(map, input, limits),
        SimpleFontMapping::ExplicitEncoding { encoding, .. } => {
            decode_encoded_text(encoding, input, limits)
        }
    }
}
