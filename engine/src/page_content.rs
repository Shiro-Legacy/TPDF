// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded loading of logical page-content bytes, without interpreting them.
//!
//! MuPDF 1.28.1's pdf_open_object_array uses fz_open_concat(..., 1), whose
//! next_concat inserts one ASCII space after EVERY member (including the final
//! or an empty member). A single stream receives no padding. This module
//! preserves that exact policy; replacing padding with newlines would change
//! cross-stream comments and literal strings. No Form/resource expansion occurs.

use std::fmt;
use std::ops::Range;

use crate::document::{Document, DocumentError};
use crate::object::{Object, ObjectId};
use crate::pages::PageInfo;
use crate::stream::{DecodeUsage, StreamLimits};
use crate::stream_object::{StreamObjectError, decode_document_stream_with_usage};

#[derive(Clone, Copy, Debug)]
pub struct PageContentLimits {
    pub max_streams: usize,
    /// Repeated references count repeatedly, even when they name one object.
    pub max_referenced_encoded_bytes: usize,
    /// Every filter/predictor output, including intermediate outputs.
    pub max_decoded_work_bytes: usize,
    /// Retained joined bytes, including inserted array-member padding. Decoder
    /// temporaries are additionally bounded by `stream` and the work budget.
    pub max_joined_bytes: usize,
    pub stream: StreamLimits,
}
impl Default for PageContentLimits {
    fn default() -> Self {
        Self {
            max_streams: 10_000,
            max_referenced_encoded_bytes: 64 * 1024 * 1024,
            max_decoded_work_bytes: 128 * 1024 * 1024,
            max_joined_bytes: 64 * 1024 * 1024,
            stream: StreamLimits::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentSegment {
    pub object: ObjectId,
    /// Logical byte range of decoded data, excluding inserted padding.
    pub range: Range<usize>,
    /// Offset of the inserted ASCII space for an array member.
    pub padding: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageContent {
    pub bytes: Vec<u8>,
    pub segments: Vec<ContentSegment>,
    pub usage: DecodeUsage,
}

#[derive(Debug)]
pub struct PageContentError {
    pub page: ObjectId,
    pub kind: PageContentErrorKind,
}
#[derive(Debug)]
pub enum PageContentErrorKind {
    Document(DocumentError),
    Stream(StreamObjectError),
    Invalid(&'static str),
    LimitExceeded(&'static str),
    ReferenceCycle(ObjectId),
    AllocationFailed,
}
impl fmt::Display for PageContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PDF page content {} {}: ",
            self.page.number(),
            self.page.generation()
        )?;
        match &self.kind {
            PageContentErrorKind::Document(source) => write!(f, "{source}"),
            PageContentErrorKind::Stream(source) => write!(f, "{source}"),
            PageContentErrorKind::Invalid(reason) => write!(f, "invalid {reason}"),
            PageContentErrorKind::LimitExceeded(reason) => write!(f, "{reason} limit exceeded"),
            PageContentErrorKind::ReferenceCycle(id) => write!(
                f,
                "Contents reference cycle at {} {}",
                id.number(),
                id.generation()
            ),
            PageContentErrorKind::AllocationFailed => {
                f.write_str("joined content allocation failed")
            }
        }
    }
}
impl std::error::Error for PageContentError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            PageContentErrorKind::Document(source) => Some(source),
            PageContentErrorKind::Stream(source) => Some(source),
            _ => None,
        }
    }
}

struct Loader<'a, 'b> {
    document: &'a Document<'b>,
    page: &'a PageInfo,
}
impl Loader<'_, '_> {
    fn error(&self, kind: PageContentErrorKind) -> PageContentError {
        PageContentError {
            page: self.page.id,
            kind,
        }
    }

    /// Validate the supplied inventory against this document and preserve the
    /// array-of-one distinction that PageInfo alone cannot represent.
    fn is_array(&self, max_streams: usize) -> Result<bool, PageContentError> {
        let page = self
            .document
            .object(self.page.id)
            .map_err(|source| self.error(PageContentErrorKind::Document(source)))?;
        if page.stream.is_some() {
            return Err(self.error(PageContentErrorKind::Invalid("page object is a stream")));
        }
        let Object::Dictionary(dictionary) = &page.value else {
            return Err(self.error(PageContentErrorKind::Invalid("page dictionary")));
        };
        let Some(mut value) = dictionary.get(b"Contents".as_slice()) else {
            return if self.page.contents.is_empty() {
                Ok(false)
            } else {
                Err(self.error(PageContentErrorKind::Invalid(
                    "PageInfo contents do not match document",
                )))
            };
        };
        if let Object::Reference(id) = value {
            if self
                .document
                .object(*id)
                .map_err(|source| self.error(PageContentErrorKind::Document(source)))?
                .stream
                .is_some()
            {
                return if self.page.contents.as_slice() == [*id] {
                    Ok(false)
                } else {
                    Err(self.error(PageContentErrorKind::Invalid(
                        "PageInfo contents do not match document",
                    )))
                };
            }
        }
        let mut seen = [None; 64];
        let mut count = 0;
        while let Object::Reference(id) = value {
            if seen[..count].contains(&Some(*id)) {
                return Err(self.error(PageContentErrorKind::ReferenceCycle(*id)));
            }
            if count == seen.len() {
                return Err(self.error(PageContentErrorKind::LimitExceeded(
                    "Contents reference depth",
                )));
            }
            seen[count] = Some(*id);
            count += 1;
            value = &self
                .document
                .object(*id)
                .map_err(|source| self.error(PageContentErrorKind::Document(source)))?
                .value;
        }
        match value {
            Object::Null if self.page.contents.is_empty() => Ok(false),
            Object::Array(values) => {
                if values.len() > max_streams {
                    return Err(self.error(PageContentErrorKind::LimitExceeded(
                        "content stream references",
                    )));
                }
                if values.len() != self.page.contents.len()
                    || values.iter().zip(&self.page.contents).any(
                        |(value, id)| !matches!(value, Object::Reference(actual) if actual == id),
                    )
                {
                    return Err(self.error(PageContentErrorKind::Invalid(
                        "PageInfo contents do not match document",
                    )));
                }
                Ok(true)
            }
            _ => Err(self.error(PageContentErrorKind::Invalid(
                "Contents must match a stream, stream array or null",
            ))),
        }
    }
}

pub fn load_page_content(
    document: &Document<'_>,
    page: &PageInfo,
    limits: PageContentLimits,
) -> Result<PageContent, PageContentError> {
    load_page_content_with_allocation(document, page, limits, usize::MAX, &mut 0)
}

/// Full requested allocation accounting for the retained join and segment arenas.
pub(crate) fn load_page_content_with_allocation(
    document: &Document<'_>,
    page: &PageInfo,
    limits: PageContentLimits,
    allocation_limit: usize,
    allocated: &mut usize,
) -> Result<PageContent, PageContentError> {
    let loader = Loader { document, page };
    let mut request = |amount: usize| -> Result<(), PageContentError> {
        *allocated = allocated
            .checked_add(amount)
            .filter(|n| *n <= allocation_limit)
            .ok_or_else(|| {
                loader.error(PageContentErrorKind::LimitExceeded(
                    "requested allocation bytes",
                ))
            })?;
        Ok(())
    };
    if page.contents.len() > limits.max_streams {
        return Err(loader.error(PageContentErrorKind::LimitExceeded(
            "content stream references",
        )));
    }
    let is_array = loader.is_array(limits.max_streams)?;
    if is_array && page.contents.len() > limits.max_joined_bytes {
        return Err(loader.error(PageContentErrorKind::LimitExceeded("joined content bytes")));
    }
    let mut bytes = Vec::new();
    let mut segments = Vec::new();
    request(
        page.contents
            .len()
            .checked_mul(std::mem::size_of::<ContentSegment>())
            .ok_or_else(|| {
                loader.error(PageContentErrorKind::LimitExceeded("segment allocation"))
            })?,
    )?;
    segments
        .try_reserve_exact(page.contents.len())
        .map_err(|_| loader.error(PageContentErrorKind::AllocationFailed))?;
    let mut usage = DecodeUsage {
        encoded_bytes: 0,
        total_decoded_bytes: 0,
    };
    for &object in &page.contents {
        let raw = document
            .stream_data(object)
            .map_err(|source| loader.error(PageContentErrorKind::Document(source)))?
            .ok_or_else(|| {
                loader.error(PageContentErrorKind::Invalid(
                    "Contents member is not a stream",
                ))
            })?;
        let next_encoded = usage
            .encoded_bytes
            .checked_add(raw.len())
            .filter(|&count| count <= limits.max_referenced_encoded_bytes)
            .ok_or_else(|| {
                loader.error(PageContentErrorKind::LimitExceeded(
                    "referenced encoded bytes",
                ))
            })?;
        let mut stream_limits = limits.stream;
        stream_limits.max_encoded_bytes = stream_limits.max_encoded_bytes.min(
            limits
                .max_referenced_encoded_bytes
                .saturating_sub(usage.encoded_bytes),
        );
        stream_limits.max_total_decoded_bytes = stream_limits.max_total_decoded_bytes.min(
            limits
                .max_decoded_work_bytes
                .saturating_sub(usage.total_decoded_bytes),
        );
        let decoded = decode_document_stream_with_usage(document, object, stream_limits)
            .map_err(|source| loader.error(PageContentErrorKind::Stream(source)))?;
        if decoded.usage.encoded_bytes != raw.len() {
            return Err(loader.error(PageContentErrorKind::Invalid(
                "decoder encoded-byte accounting",
            )));
        }
        usage.encoded_bytes = next_encoded;
        usage.total_decoded_bytes = usage
            .total_decoded_bytes
            .checked_add(decoded.usage.total_decoded_bytes)
            .filter(|&count| count <= limits.max_decoded_work_bytes)
            .ok_or_else(|| {
                loader.error(PageContentErrorKind::LimitExceeded("decoded work bytes"))
            })?;
        let padding_bytes = usize::from(is_array);
        let additional = decoded
            .bytes
            .len()
            .checked_add(padding_bytes)
            .ok_or_else(|| {
                loader.error(PageContentErrorKind::LimitExceeded("joined content bytes"))
            })?;
        let end = bytes
            .len()
            .checked_add(additional)
            .filter(|&count| count <= limits.max_joined_bytes)
            .ok_or_else(|| {
                loader.error(PageContentErrorKind::LimitExceeded("joined content bytes"))
            })?;
        // Geometric growth bounds cumulative copying across many small members;
        // cap requested capacity so spare capacity respects the retained limit.
        if end > bytes.capacity() {
            let target = end
                .max(bytes.capacity().saturating_mul(2).max(64))
                .min(limits.max_joined_bytes);
            request(target)?;
            bytes
                .try_reserve_exact(target - bytes.len())
                .map_err(|_| loader.error(PageContentErrorKind::AllocationFailed))?;
        }
        let start = bytes.len();
        bytes.extend_from_slice(&decoded.bytes);
        let range = start..bytes.len();
        let padding = if is_array {
            let at = bytes.len();
            bytes.push(b' ');
            Some(at)
        } else {
            None
        };
        debug_assert_eq!(bytes.len(), end);
        segments.push(ContentSegment {
            object,
            range,
            padding,
        });
    }
    Ok(PageContent {
        bytes,
        segments,
        usage,
    })
}
