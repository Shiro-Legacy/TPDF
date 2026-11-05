// SPDX-License-Identifier: MIT
//! Dynamic XFA forms, laid out and materialized as an ordinary AcroForm
//! document so that the rest of the engine (view, text, search, fields,
//! save) works on them unchanged. Design: `design/app-xfa.md`.
//!
//! Pipeline: `packets` reads the XDP packets out of /AcroForm /XFA;
//! `template` reads the template packet into the `types` model; `bind`
//! expands instances and sets values from the datasets packet; `layout`
//! flows the form into pages; `emit` writes a new PDF with those pages,
//! widget annotations for the fillable fields and the XFA packets kept;
//! `sync` writes filled values back into the datasets packet. `text` and
//! `richtext` measure and wrap text for `layout` and `emit`.
pub mod bind;
pub mod emit;
pub mod layout;
pub mod packets;
pub mod richtext;
pub mod sync;
pub mod template;
pub mod text;
pub mod types;
pub mod xml;

use std::fmt;

use crate::document::Document;

/// The catalog key a materialized document carries (its value the
/// materializer's version), so a reopen keeps the pages it has.
pub const MARKER_KEY: &[u8] = b"TPDFXfa";
/// The materializer's version, written as `MARKER_KEY`'s value.
pub const MARKER_VERSION: i64 = 1;
/// The private key on a materialized terminal field holding the
/// `DataPath` (as `Display` writes it) its value is written back to.
pub const DATA_KEY: &[u8] = b"TPDFData";

/// Why a form could not be materialized. The caller falls back to the
/// document as it is (its placeholder page) on any of these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XfaError {
    /// No /AcroForm /XFA, or none readable.
    NoXfa,
    /// A packet's XML could not be read; the message says which and why.
    Xml(String),
    /// The template is not one this renderer lays out (the message says what).
    Unsupported(String),
    /// A bound the renderer enforces was passed (the message names it).
    Limit(&'static str),
    /// The materialized document could not be written.
    Write(String),
}

impl fmt::Display for XfaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XfaError::NoXfa => f.write_str("no XFA packets"),
            XfaError::Xml(m) => write!(f, "XFA XML: {m}"),
            XfaError::Unsupported(m) => write!(f, "XFA unsupported: {m}"),
            XfaError::Limit(m) => write!(f, "XFA limit: {m}"),
            XfaError::Write(m) => write!(f, "XFA write: {m}"),
        }
    }
}

impl std::error::Error for XfaError {}

/// Limits the renderer enforces on hostile input.
pub mod limits {
    /// Bytes of every packet read, together.
    pub const MAX_PACKET_BYTES: usize = 64 << 20;
    /// Elements in one packet's XML.
    pub const MAX_XML_ELEMENTS: usize = 2_000_000;
    /// XML nesting depth.
    pub const MAX_XML_DEPTH: usize = 512;
    /// Nodes of the bound form after instance expansion.
    pub const MAX_FORM_NODES: usize = 200_000;
    /// Pages the layout produces.
    pub const MAX_PAGES: usize = 2_000;
    /// Items on all pages together.
    pub const MAX_ITEMS: usize = 1_000_000;
    /// Characters laid out in one text block.
    pub const MAX_BLOCK_CHARS: usize = 1 << 20;
}

/// Whether `doc` is a dynamic XFA form the materializer should lay out:
/// it has /AcroForm /XFA, the catalog says /NeedsRendering true, and it
/// does not carry `MARKER_KEY` already.
pub fn is_dynamic(doc: &Document<'_>) -> bool {
    packets::is_dynamic(doc)
}

/// Lays out `doc`'s XFA form and writes a new PDF holding the laid-out
/// pages, widget annotations for the fillable fields, the XFA packets
/// (the `form` packet dropped) and `MARKER_KEY`. Fails rather than
/// producing a document with nothing on it; the caller then shows the
/// original.
pub fn materialize(doc: &Document<'_>) -> Result<Vec<u8>, XfaError> {
    let packets = packets::read(doc)?;
    let mut form = template::read(&packets.template)?;
    let datasets = match &packets.datasets {
        Some(text) => Some(xml::parse(text).map_err(|e| XfaError::Xml(format!("datasets: {e}")))?),
        None => None,
    };
    bind::bind(&mut form, datasets.as_ref())?;
    let pages = layout::lay_out(&form)?;
    emit::materialize(doc, &packets, &form, &pages)
}
