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
//! `script` runs the form's JavaScript events (`initialize` at open, a
//! field's handlers on a fill) over the bound form before layout; `sync`
//! writes filled values back into the datasets packet. `text` and
//! `richtext` measure and wrap text for `layout` and `emit`.
pub mod bind;
pub mod emit;
pub mod layout;
pub mod packets;
pub mod richtext;
pub mod script;
pub mod sync;
pub mod template;
pub mod text;
pub mod types;
pub mod xml;

use std::collections::HashMap;
use std::fmt;

use crate::document::Document;
use types::{DataPath, Form, Node};

/// The catalog key a materialized document carries (its value the
/// materializer's version), so a reopen keeps the pages it has.
pub const MARKER_KEY: &[u8] = b"TPDFXfa";
/// The materializer's version, written as `MARKER_KEY`'s value.
pub const MARKER_VERSION: i64 = 1;
/// The private key on a materialized terminal field holding the
/// `DataPath` (as `Display` writes it) its value is written back to.
pub const DATA_KEY: &[u8] = b"TPDFData";
/// The catalog key holding `signature` of the scripted state the pages
/// were laid out from (an integer: the hash's bits).
pub const SIGNATURE_KEY: &[u8] = b"TPDFXfaSig";

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

/// What `materialize_after_fill` found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Materialized {
    /// A new document: the fill changed what the scripts show or set.
    Written(Vec<u8>),
    /// The fill changed nothing but its own value: the pages stand.
    Unchanged,
}

/// Lays out `doc`'s XFA form and writes a new PDF holding the laid-out
/// pages, widget annotations for the fillable fields, the XFA packets
/// (the `form` packet dropped, the datasets with the values the scripts
/// set) and `MARKER_KEY`. The form's `initialize` and `ready` events run
/// first, then its `calculate` scripts (`script`). Fails rather than
/// producing a document with nothing on it; the caller then shows the
/// original.
pub fn materialize(doc: &Document<'_>) -> Result<Vec<u8>, XfaError> {
    match build(doc, None)? {
        Materialized::Written(bytes) => Ok(bytes),
        Materialized::Unchanged => Err(XfaError::Write("nothing to write".into())),
    }
}

/// `materialize` of a document this app materialized, after the field
/// bound to `path` was filled with `value` (its datasets packet already
/// updated): the `initialize` events and calculations run on the new
/// data, then the filled node's handlers and the calculations again.
/// `Unchanged` when the scripted state (`signature`) is the one the
/// document was laid out from and no field value was set by a script, so
/// the pages and widgets can stay.
pub fn materialize_after_fill(
    doc: &Document<'_>,
    path: &DataPath,
    value: &str,
) -> Result<Materialized, XfaError> {
    build(doc, Some((path, value)))
}

fn build(doc: &Document<'_>, fill: Option<(&DataPath, &str)>) -> Result<Materialized, XfaError> {
    let packets = packets::read(doc)?;
    let mut form = template::read(&packets.template)?;
    let datasets = match &packets.datasets {
        Some(text) => Some(xml::parse(text).map_err(|e| XfaError::Xml(format!("datasets: {e}")))?),
        None => None,
    };
    bind::bind(&mut form, datasets.as_ref())?;
    let before = values(&form);
    {
        let mut runner = script::Runner::new(&mut form);
        runner.initialize();
        runner.calculate();
        if let Some((path, value)) = fill {
            runner.fire(path, value);
            runner.calculate();
        }
    }
    bind::record_ids(&mut form);
    let (changed, fillable_changed) = changed_values(&form, &before);
    let signature = signature(&form);
    if fill.is_some() && !fillable_changed && stored_signature(doc) == Some(signature) {
        return Ok(Materialized::Unchanged);
    }
    let mut extra = emit::Extra {
        datasets: None,
        signature: Some(signature),
    };
    if let (Some(text), false) = (&packets.datasets, changed.is_empty()) {
        if packets.whole.is_none() {
            if let Ok(updated) = sync::update_datasets(text, &changed) {
                extra.datasets = Some(updated);
            }
        }
    }
    let pages = layout::lay_out(&form)?;
    emit::materialize_with(doc, &packets, &form, &pages, &extra).map(Materialized::Written)
}

/// Every field's and group's value by SOM name.
fn values(form: &Form) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut stack: Vec<&Node> = form.root.children.iter().collect();
    while let Some(node) = stack.pop() {
        match node {
            Node::Subform(s) => stack.extend(s.children.iter()),
            Node::Field(f) => {
                out.insert(f.som.clone(), f.value.plain());
            }
            Node::ExclGroup(g) => {
                out.insert(g.som.clone(), g.value.clone());
            }
            Node::Draw(_) => {}
        }
    }
    out
}

/// The values the scripts changed, as datasets updates for the bound
/// nodes (in document order), and whether any of them is a fillable
/// field's (one with a widget whose /V would be stale).
fn changed_values(
    form: &Form,
    before: &HashMap<String, String>,
) -> (Vec<(DataPath, String)>, bool) {
    let mut out = Vec::new();
    let mut fillable = false;
    let mut stack: Vec<&Node> = form.root.children.iter().rev().collect();
    while let Some(node) = stack.pop() {
        let (som, value, data, is_fillable) = match node {
            Node::Subform(s) => {
                stack.extend(s.children.iter().rev());
                continue;
            }
            Node::Field(f) => (&f.som, f.value.plain(), &f.data, f.fillable()),
            Node::ExclGroup(g) => (&g.som, g.value.clone(), &g.data, !g.common.read_only),
            Node::Draw(_) => continue,
        };
        if before.get(som).is_some_and(|old| *old == value) {
            continue;
        }
        fillable |= is_fillable;
        if let Some(path) = data {
            out.push((path.clone(), value));
        }
    }
    (out, fillable)
}

/// A hash of what the scripts can change that the layout shows: every
/// node's presence and access, the values of draws and of fields that are
/// not fillable or are floating fields (embedded in rich text). Fillable
/// fields' values are left out: a fill changes them without changing the
/// layout.
pub fn signature(form: &Form) -> u64 {
    let mut hash = Fnv::default();
    let mut stack: Vec<&Node> = form.root.children.iter().rev().collect();
    hash.bytes(form.root.common.name.as_bytes());
    while let Some(node) = stack.pop() {
        let common = node.common();
        hash.bytes(common.name.as_bytes());
        hash.bytes(&[common.presence as u8, u8::from(common.read_only)]);
        match node {
            Node::Subform(s) => {
                hash.bytes(b"s");
                stack.extend(s.children.iter().rev());
            }
            Node::Field(f) => {
                hash.bytes(b"f");
                if !f.fillable() || !f.common.id.is_empty() {
                    hash.bytes(f.value.plain().as_bytes());
                }
            }
            Node::Draw(d) => {
                hash.bytes(b"d");
                hash.bytes(d.value.plain().as_bytes());
            }
            Node::ExclGroup(g) => {
                hash.bytes(b"g");
                for f in &g.fields {
                    hash.bytes(f.common.name.as_bytes());
                    hash.bytes(&[f.common.presence as u8, u8::from(f.common.read_only)]);
                }
            }
        }
        hash.bytes(b"\0");
    }
    hash.0
}

/// `SIGNATURE_KEY` of `doc`'s catalog.
pub fn stored_signature(doc: &Document<'_>) -> Option<u64> {
    let catalog = doc.resolve_id(doc.root()).as_dict()?;
    doc.dict_get(catalog, SIGNATURE_KEY)
        .as_i64()
        .map(|bits| bits as u64)
}

/// FNV-1a, 64 bits: a stable hash across builds.
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.0 ^= 0xff;
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
    }
}
