// SPDX-License-Identifier: AGPL-3.0-or-later
//! Dictionary-aware stream decoding. Only full PDF stream filter names are
//! accepted; inline-image abbreviations are not aliases in this API. Indirect
//! metadata references are followed for at most 64 hops with cycle checks.
//! Decode parameters are strict: null/empty dictionaries are accepted, and
//! Flate and LZW support TIFF predictor 2 and PNG predictors 10..=15, with Colors,
//! BitsPerComponent, and Columns. Predictor 1 is identity. Other non-null
//! entries fail explicitly. Predictor component depths are 1/2/4/8/16 bits.
//! LZW additionally accepts integer EarlyChange 0 or 1 (default 1). Budgets apply per call; callers decoding many
//! streams must also impose their own document-wide output/work budget.

use std::fmt;

use crate::document::{Document, DocumentError};
use crate::object::{Dictionary, Object, ObjectId};
use crate::stream::{
    DecodedStream, Filter, FilterSpec, PredictorParams, StreamError, StreamLimits,
    decode_filter_specs_lenient, decode_filter_specs_with_usage,
};

#[derive(Debug)]
pub struct StreamObjectError {
    pub object: ObjectId,
    pub kind: StreamObjectErrorKind,
}

#[derive(Debug)]
pub enum StreamObjectErrorKind {
    Document(DocumentError),
    NotStream,
    Invalid(&'static str),
    Unsupported(&'static str),
    UnknownFilter(Vec<u8>),
    ReferenceCycle(ObjectId),
    ReferenceLimit,
    FilterLimit { limit: usize },
    AllocationFailed,
    Decode(StreamError),
}

impl fmt::Display for StreamObjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PDF stream object {} {}: ",
            self.object.number(),
            self.object.generation()
        )?;
        match &self.kind {
            StreamObjectErrorKind::Document(error) => write!(f, "{error}"),
            StreamObjectErrorKind::NotStream => f.write_str("object has no embedded stream"),
            StreamObjectErrorKind::Invalid(reason) => write!(f, "invalid {reason}"),
            StreamObjectErrorKind::Unsupported(reason) => write!(f, "unsupported {reason}"),
            StreamObjectErrorKind::UnknownFilter(name) => {
                f.write_str("unsupported filter name /")?;
                for &b in name {
                    if b.is_ascii_graphic() && !b"#()<>[]{}/%".contains(&b) {
                        write!(f, "{}", char::from(b))?;
                    } else {
                        write!(f, "#{b:02x}")?;
                    }
                }
                Ok(())
            }
            StreamObjectErrorKind::ReferenceCycle(id) => write!(
                f,
                "metadata reference cycle at {} {}",
                id.number(),
                id.generation()
            ),
            StreamObjectErrorKind::ReferenceLimit => {
                f.write_str("metadata exceeds 64 reference hops")
            }
            StreamObjectErrorKind::FilterLimit { limit } => {
                write!(f, "filter chain exceeds {limit} stages")
            }
            StreamObjectErrorKind::AllocationFailed => {
                f.write_str("filter metadata allocation failed")
            }
            StreamObjectErrorKind::Decode(error) => write!(f, "{error}"),
        }
    }
}
impl std::error::Error for StreamObjectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            StreamObjectErrorKind::Document(error) => Some(error),
            StreamObjectErrorKind::Decode(error) => Some(error),
            _ => None,
        }
    }
}

struct Metadata<'d, 'i> {
    document: &'d Document<'i>,
    stream_id: ObjectId,
}
impl<'d, 'i> Metadata<'d, 'i> {
    fn error(&self, kind: StreamObjectErrorKind) -> StreamObjectError {
        StreamObjectError {
            object: self.stream_id,
            kind,
        }
    }

    fn resolve(&self, mut value: &'d Object) -> Result<&'d Object, StreamObjectError> {
        let mut seen = [None; 64];
        let mut count = 0;
        while let Object::Reference(id) = value {
            if seen[..count].contains(&Some(*id)) {
                return Err(self.error(StreamObjectErrorKind::ReferenceCycle(*id)));
            }
            if count == seen.len() {
                return Err(self.error(StreamObjectErrorKind::ReferenceLimit));
            }
            seen[count] = Some(*id);
            count += 1;
            let target = self
                .document
                .object(*id)
                .map_err(|error| self.error(StreamObjectErrorKind::Document(error)))?;
            if target.stream.is_some() {
                return Err(self.error(StreamObjectErrorKind::Invalid(
                    "metadata reference targets a stream",
                )));
            }
            value = &target.value;
        }
        Ok(value)
    }

    fn filter(&self, value: &'d Object) -> Result<Filter, StreamObjectError> {
        let Object::Name(name) = self.resolve(value)? else {
            return Err(self.error(StreamObjectErrorKind::Invalid(
                "/Filter entries must resolve to names",
            )));
        };
        match name.as_slice() {
            b"FlateDecode" => Ok(Filter::Flate),
            b"ASCIIHexDecode" => Ok(Filter::AsciiHex),
            b"ASCII85Decode" => Ok(Filter::Ascii85),
            b"RunLengthDecode" => Ok(Filter::RunLength),
            b"LZWDecode" => Ok(Filter::Lzw { early_change: true }),
            _ => Err(self.error(StreamObjectErrorKind::UnknownFilter(name.clone()))),
        }
    }

    fn parameters(
        &self,
        mut filter: Filter,
        value: &'d Object,
    ) -> Result<(Filter, PredictorParams), StreamObjectError> {
        let mut parameters = PredictorParams::default();
        let dictionary = match self.resolve(value)? {
            Object::Null => return Ok((filter, parameters)),
            Object::Dictionary(dictionary) => dictionary,
            _ => {
                return Err(self.error(StreamObjectErrorKind::Invalid(
                    "/DecodeParms entries must resolve to dictionaries or null",
                )));
            }
        };
        // In key order, so an error does not depend on source order.
        for (key, value) in dictionary.sorted_entries() {
            let value = self.resolve(value)?;
            if matches!(value, Object::Null) {
                continue;
            }
            if key == b"EarlyChange" && matches!(filter, Filter::Lzw { .. }) {
                filter = match value {
                    Object::Integer(0) => Filter::Lzw {
                        early_change: false,
                    },
                    Object::Integer(1) => Filter::Lzw { early_change: true },
                    Object::Integer(_) => {
                        return Err(self.error(StreamObjectErrorKind::Unsupported(
                            "LZW EarlyChange must be 0 or 1",
                        )));
                    }
                    _ => {
                        return Err(self.error(StreamObjectErrorKind::Invalid(
                            "LZW EarlyChange must be an integer",
                        )));
                    }
                };
                continue;
            }
            if !matches!(filter, Filter::Flate | Filter::Lzw { .. })
                || !matches!(
                    key.as_slice(),
                    b"Predictor" | b"Colors" | b"BitsPerComponent" | b"Columns"
                )
            {
                return Err(self.error(StreamObjectErrorKind::Unsupported(
                    "non-null decode parameter",
                )));
            }
            let Object::Integer(number) = value else {
                return Err(self.error(StreamObjectErrorKind::Invalid(
                    "predictor parameters must be integers",
                )));
            };
            match key.as_slice() {
                b"Predictor" => {
                    parameters.predictor = u8::try_from(*number).map_err(|_| {
                        self.error(StreamObjectErrorKind::Unsupported(
                            "predictor value outside 0..=255",
                        ))
                    })?
                }
                b"BitsPerComponent" => {
                    parameters.bits_per_component = u8::try_from(*number).map_err(|_| {
                        self.error(StreamObjectErrorKind::Unsupported(
                            "predictor bit depth outside 0..=255",
                        ))
                    })?
                }
                b"Colors" => {
                    parameters.colors = usize::try_from(*number).map_err(|_| {
                        self.error(StreamObjectErrorKind::Invalid(
                            "negative or oversized Colors",
                        ))
                    })?
                }
                b"Columns" => {
                    parameters.columns = usize::try_from(*number).map_err(|_| {
                        self.error(StreamObjectErrorKind::Invalid(
                            "negative or oversized Columns",
                        ))
                    })?
                }
                _ => unreachable!(),
            }
        }
        Ok((filter, parameters))
    }

    fn filters(
        &self,
        dictionary: &'d Dictionary,
        max_filters: usize,
    ) -> Result<Vec<FilterSpec>, StreamObjectError> {
        let value = dictionary
            .get(b"Filter".as_slice())
            .map(|value| self.resolve(value))
            .transpose()?;
        let count = match value {
            None | Some(Object::Null) => 0,
            Some(Object::Name(_)) => 1,
            Some(Object::Array(values)) => values.len(),
            _ => {
                return Err(self.error(StreamObjectErrorKind::Invalid(
                    "/Filter must resolve to a name, array or null",
                )));
            }
        };
        if count > max_filters {
            return Err(self.error(StreamObjectErrorKind::FilterLimit { limit: max_filters }));
        }
        let mut filters = Vec::new();
        filters
            .try_reserve_exact(count)
            .map_err(|_| self.error(StreamObjectErrorKind::AllocationFailed))?;
        match value {
            Some(value @ Object::Name(_)) => filters.push(self.filter(value)?.into()),
            Some(Object::Array(values)) => {
                for value in values {
                    filters.push(self.filter(value)?.into());
                }
            }
            _ => {}
        }
        Ok(filters)
    }
}

/// Decode to final bytes, retaining the original API for callers that manage
/// only one stream. Multi-stream loaders should use the usage-returning API.
pub fn decode_document_stream(
    document: &Document<'_>,
    id: ObjectId,
    limits: StreamLimits,
) -> Result<Vec<u8>, StreamObjectError> {
    decode_document_stream_with_usage(document, id, limits).map(|decoded| decoded.bytes)
}

/// Decode one stream and report actual cumulative intermediate output work.
pub fn decode_document_stream_with_usage(
    document: &Document<'_>,
    id: ObjectId,
    limits: StreamLimits,
) -> Result<DecodedStream, StreamObjectError> {
    decode_document_stream_as(document, id, limits, false)
}

/// `decode_document_stream` with tolerant Flate decoding
/// (`stream::decode_filter_specs_lenient`): bytes after the end of the zlib
/// stream are ignored, and a truncated or corrupt stream or a bad checksum
/// keeps the bytes produced before it. Metadata is read as strictly.
pub fn decode_document_stream_lenient(
    document: &Document<'_>,
    id: ObjectId,
    limits: StreamLimits,
) -> Result<Vec<u8>, StreamObjectError> {
    decode_document_stream_as(document, id, limits, true).map(|decoded| decoded.bytes)
}

fn decode_document_stream_as(
    document: &Document<'_>,
    id: ObjectId,
    limits: StreamLimits,
    lenient: bool,
) -> Result<DecodedStream, StreamObjectError> {
    let metadata = Metadata {
        document,
        stream_id: id,
    };
    let object = document
        .object(id)
        .map_err(|error| metadata.error(StreamObjectErrorKind::Document(error)))?;
    let data = document
        .stream_data(id)
        .map_err(|error| metadata.error(StreamObjectErrorKind::Document(error)))?
        .ok_or_else(|| metadata.error(StreamObjectErrorKind::NotStream))?;
    let Object::Dictionary(dictionary) = &object.value else {
        return Err(metadata.error(StreamObjectErrorKind::Invalid("stream dictionary")));
    };
    // An external file specification (/F, /FFilter, /FDecodeParms) is
    // ignored, as reference viewers ignore it: such a stream exists only in a document
    // opened with `OpenOptions::external_streams`, and its own bytes decode.
    let mut filters = metadata.filters(dictionary, limits.max_filters)?;
    let parameters = dictionary
        .get(b"DecodeParms".as_slice())
        .map(|value| metadata.resolve(value))
        .transpose()?;
    match parameters {
        None | Some(Object::Null) => {}
        Some(_) if filters.is_empty() => {
            return Err(metadata.error(StreamObjectErrorKind::Invalid(
                "non-null /DecodeParms without filters",
            )));
        }
        Some(value @ Object::Dictionary(_)) if filters.len() == 1 => {
            let (filter, predictor) = metadata.parameters(filters[0].filter, value)?;
            filters[0] = FilterSpec { filter, predictor };
        }
        Some(Object::Array(values)) if values.len() == filters.len() => {
            for (spec, value) in filters.iter_mut().zip(values) {
                let (filter, predictor) = metadata.parameters(spec.filter, value)?;
                *spec = FilterSpec { filter, predictor };
            }
        }
        Some(Object::Array(_)) => {
            return Err(metadata.error(StreamObjectErrorKind::Invalid(
                "/DecodeParms array length must equal /Filter array length",
            )));
        }
        _ => {
            return Err(metadata.error(StreamObjectErrorKind::Invalid(
                "/DecodeParms shape does not match filters",
            )));
        }
    }
    match lenient {
        true => decode_filter_specs_lenient(data, &filters, limits),
        false => decode_filter_specs_with_usage(data, &filters, limits),
    }
    .map_err(|error| metadata.error(StreamObjectErrorKind::Decode(error)))
}
