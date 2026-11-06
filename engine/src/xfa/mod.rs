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
//! writes filled values back into the datasets packet; `picture` formats
//! values for display and reads typed text back (picture clauses). `text` and
//! `richtext` measure and wrap text for `layout` and `emit`; `image` reads
//! image values (JPEG, PNG) for `emit`.
pub mod bind;
pub mod emit;
pub mod fonts;
pub mod image;
pub mod layout;
pub mod packets;
pub mod picture;
pub mod reopen;
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
/// The materializer's version, written as `MARKER_KEY`'s value. Version 2
/// writes `DATA_HASH_KEY`; version 1 files (without it) are still read
/// (`reopen`). Version 3 splits an `lr-tb` line holding one subform alone
/// across pages (`layout`); version 4 lays out again after a fill of a
/// field that grows with its text (`signature`). An older file is laid out
/// again at open (`Reason::LayoutChanged`).
pub const MARKER_VERSION: i64 = 4;
/// The private key on a materialized terminal field holding the
/// `DataPath` (as `Display` writes it) its value is written back to.
pub const DATA_KEY: &[u8] = b"TPDFData";
/// The private key on a materialized push button holding its SOM name,
/// so a click (`materialize_after_click`) finds the node whose `click`
/// event runs.
pub const CLICK_KEY: &[u8] = b"TPDFSom";
/// The catalog key holding `signature` of the scripted state the pages
/// were laid out from (an integer: the hash's bits).
pub const SIGNATURE_KEY: &[u8] = b"TPDFXfaSig";
/// The catalog key holding `data_hash` of the datasets packet the
/// document was written with (and kept by this app's fills), so a reopen
/// tells a fill by another XFA processor (`reopen`).
pub const DATA_HASH_KEY: &[u8] = b"TPDFXfaData";

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
    /// Instances `instanceManager` keeps of one repeating subform, at most
    /// (the `occur` max when it is lower).
    pub const MAX_INSTANCES: usize = 1024;
    /// `instanceManager` operations one materialization applies, at most.
    pub const MAX_INSTANCE_OPS: usize = 1024;
}

/// Whether `doc` is a dynamic XFA form the materializer should lay out:
/// it has /AcroForm /XFA, the catalog says /NeedsRendering true, and it
/// does not carry `MARKER_KEY` already.
pub fn is_dynamic(doc: &Document<'_>) -> bool {
    packets::is_dynamic(doc)
}

pub use reopen::{Reason, needs_rebuild, rebuild_at_open};

/// What `materialize_after_fill` and `materialize_after_click` found,
/// with the messages the handlers recorded for the user either way
/// (`xfa.host.messageBox`, failed validations: `script::Report::messages`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Materialized {
    /// A new document: the fill changed what the scripts show or set.
    Written {
        bytes: Vec<u8>,
        messages: Vec<String>,
    },
    /// The fill changed nothing but its own value: the pages stand.
    Unchanged { messages: Vec<String> },
}

impl Materialized {
    /// The messages the handlers recorded, in order.
    pub fn messages(&self) -> &[String] {
        match self {
            Materialized::Written { messages, .. } | Materialized::Unchanged { messages } => {
                messages
            }
        }
    }
}

/// Lays out `doc`'s XFA form and writes a new PDF holding the laid-out
/// pages, widget annotations for the fillable fields, the XFA packets
/// (the `form` packet dropped, the datasets with the values the scripts
/// set) and `MARKER_KEY`. The form's `initialize` and `ready` events run
/// first, then its `calculate` scripts (`script`). Fails rather than
/// producing a document with nothing on it; the caller then shows the
/// original.
pub fn materialize(doc: &Document<'_>) -> Result<Vec<u8>, XfaError> {
    match build(doc, Trigger::Open)? {
        Materialized::Written { bytes, .. } => Ok(bytes),
        Materialized::Unchanged { .. } => Err(XfaError::Write("nothing to write".into())),
    }
}

/// `materialize` of a document this app materialized, after the field
/// bound to `path` was filled with `value` (its datasets packet already
/// updated): the `initialize` events and calculations run on the new
/// data, then the filled node's handlers and the calculations again.
/// `Unchanged` when the scripted state (`signature`) is the one the
/// document was laid out from and no field value was set by a script, so
/// the pages and widgets can stay. A handler's `instanceManager`
/// operations are applied as for a click.
pub fn materialize_after_fill(
    doc: &Document<'_>,
    path: &DataPath,
    value: &str,
) -> Result<Materialized, XfaError> {
    build(doc, Trigger::Fill(path, value))
}

/// `materialize` of a document this app materialized, after a click on
/// the push button whose SOM name is `som` (its widget's `CLICK_KEY`): the
/// `initialize` events and calculations run, then the button's `click`
/// handlers. The `instanceManager` operations they asked for (add row,
/// delete row) are applied to the datasets packet, and the form is bound
/// to the result and its `initialize` events and calculations run once
/// more (operations those ask for are dropped) before the layout.
/// `Unchanged` as for a fill when the click changed nothing.
pub fn materialize_after_click(doc: &Document<'_>, som: &str) -> Result<Materialized, XfaError> {
    build(doc, Trigger::Click(som))
}

/// What a build runs the form's handlers for.
#[derive(Clone, Copy)]
enum Trigger<'a> {
    /// The open: the `initialize` and `ready` events and calculations only.
    Open,
    /// A fill of the node bound to this path with this value.
    Fill(&'a DataPath, &'a str),
    /// A click on the push button with this SOM name.
    Click(&'a str),
}

fn build(doc: &Document<'_>, trigger: Trigger<'_>) -> Result<Materialized, XfaError> {
    build_from(doc, packets::read(doc)?, false, trigger)
}

/// `build` from `packets` as read from `doc`; with `data_written`, its
/// datasets text (changed from the stored packet by the caller) is written
/// whether or not the scripts change it.
fn build_from(
    doc: &Document<'_>,
    packets: packets::Packets,
    data_written: bool,
    trigger: Trigger<'_>,
) -> Result<Materialized, XfaError> {
    let mut form = read_form(&packets)?;
    let mut datasets = match &packets.datasets {
        Some(text) => Some(xml::parse(text).map_err(|e| XfaError::Xml(format!("datasets: {e}")))?),
        None => None,
    };
    bind::bind(&mut form, datasets.as_ref())?;
    let mut before = values(&form);
    // A fill's typed text read through the field's pictures: the canonical
    // value goes to the data (a change `changed_values` reports), and the
    // widget is written again when what it shows is not what was typed.
    let reformat = match trigger {
        Trigger::Fill(path, typed) => canonicalize_fill(&mut form, path, typed),
        _ => false,
    };
    // After an edit the `initialize` events run again only to rebuild what
    // they show and hide: Acrobat runs them once, at open, so the values
    // they set then are in the data already, and one that clears a field
    // (`Para.Study.rawValue = ""` while a box is unticked) must not undo
    // what the user filled since.
    let data_values = (!matches!(trigger, Trigger::Open)).then(|| bound_values(&form, None));
    let (ops, messages) = {
        let mut runner = script::Runner::new(&mut form);
        runner.initialize();
        if let Some(saved) = &data_values {
            restore_bound_values(runner.form, saved);
        }
        runner.calculate();
        // Instance operations of the open-time events are not applied: the
        // form is built again from the data at every fill, so a row an
        // `initialize` event adds would be added again each time. Their
        // messages are not shown either (they would come back at every
        // fill): only the fill's or click's own handlers speak.
        runner.take_instance_ops();
        runner.take_messages();
        let report = match trigger {
            Trigger::Open => None,
            Trigger::Fill(path, value) => Some(runner.fire(path, value)),
            Trigger::Click(som) => Some(runner.click(som)),
        };
        let ops = runner.take_instance_ops();
        if report.is_some() {
            runner.calculate();
        }
        (ops, report.map(|r| r.messages).unwrap_or_default())
    };
    // The handlers' instance operations, applied to the data (with the
    // values the scripts set so far), and the form bound to it again.
    let mut rebound = false;
    if let (false, None, Some(tree)) = (ops.is_empty(), packets.whole, datasets.as_mut()) {
        let mut updated = tree.clone();
        let (changed, _) = changed_values(&form, &before);
        let applied = sync::update_element(&mut updated, &changed)
            .and_then(|()| sync::apply_instance_ops(&mut updated, &ops));
        if matches!(applied, Ok(n) if n > 0) {
            let mut again = read_form(&packets)?;
            bind::bind(&mut again, Some(&updated))?;
            // Rows that were there keep their data; a row the operations
            // added is new, so its `initialize` events set its values.
            let saved = bound_values(&again, Some(&before));
            before = values(&again);
            {
                let mut runner = script::Runner::new(&mut again);
                runner.initialize();
                restore_bound_values(runner.form, &saved);
                runner.calculate();
            }
            form = again;
            *tree = updated;
            rebound = true;
        }
    }
    bind::record_ids(&mut form);
    let (changed, fillable_changed) = changed_values(&form, &before);
    let signature = signature(&form);
    let after_edit = !matches!(trigger, Trigger::Open);
    if after_edit
        && !rebound
        && !fillable_changed
        && !reformat
        && stored_signature(doc) == Some(signature)
    {
        return Ok(Materialized::Unchanged { messages });
    }
    let mut extra = emit::Extra {
        datasets: if data_written {
            packets.datasets.clone()
        } else {
            None
        },
        signature: Some(signature),
    };
    if let (Some(text), None) = (&packets.datasets, packets.whole) {
        if rebound {
            if let Some(tree) = datasets.as_mut() {
                if sync::update_element(tree, &changed).is_ok() {
                    extra.datasets = Some(sync::serialize_like(text, tree));
                }
            }
        } else if !changed.is_empty() {
            if let Ok(updated) = sync::update_datasets(text, &changed) {
                extra.datasets = Some(updated);
            }
        }
    }
    form.fonts = fonts::EmbeddedFonts::scan(doc).map(std::sync::Arc::new);
    let pages = layout::lay_out(&form)?;
    let bytes = emit::materialize_with(doc, &packets, &form, &pages, &extra)?;
    Ok(Materialized::Written { bytes, messages })
}

/// The template read, with the `localeSet` packet's locales.
fn read_form(packets: &packets::Packets) -> Result<Form, XfaError> {
    let mut form = template::read(&packets.template)?;
    form.locales = picture::LocaleSet::read(packets.locale_set.as_deref());
    Ok(form)
}

/// The fillable fields bound to `path` given the canonical value of
/// `typed` (`picture::canonical`, read with the first one's pictures);
/// unchanged when nothing reads it. Whether that field now shows
/// something other than `typed`.
fn canonicalize_fill(form: &mut Form, path: &DataPath, typed: &str) -> bool {
    let Some(first) = first_field_at(form, path).cloned() else {
        return false;
    };
    let Some(canonical) = picture::canonical(form, &first, typed) else {
        return false;
    };
    let value = if canonical.is_empty() {
        types::Value::Empty
    } else {
        types::Value::Text(canonical)
    };
    let mut stack: Vec<&mut Node> = form.root.children.iter_mut().collect();
    while let Some(node) = stack.pop() {
        match node {
            Node::Subform(s) => stack.extend(s.children.iter_mut()),
            Node::Field(f) if f.fillable() && f.data.as_ref() == Some(path) => {
                f.value = value.clone();
            }
            _ => {}
        }
    }
    let shown = picture::shown(form, &types::Field { value, ..first });
    shown != typed
}

/// The first fillable field bound to `path`, in document order.
fn first_field_at<'f>(form: &'f Form, path: &DataPath) -> Option<&'f types::Field> {
    let mut stack: Vec<&Node> = form.root.children.iter().rev().collect();
    while let Some(node) = stack.pop() {
        match node {
            Node::Subform(s) => stack.extend(s.children.iter().rev()),
            Node::Field(f) if f.fillable() && f.data.as_ref() == Some(path) => return Some(f),
            _ => {}
        }
    }
    None
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

/// What a field or group bound to data holds.
#[derive(Clone)]
enum BoundValue {
    /// The value and, for a check button, whether it is on.
    Field(types::Value, bool),
    /// The group's value and its buttons'.
    Group(String, Vec<(types::Value, bool)>),
}

/// The values of the fields and groups bound to data, by SOM name; with
/// `only`, of those whose SOM name it has.
fn bound_values(
    form: &Form,
    only: Option<&HashMap<String, String>>,
) -> HashMap<String, BoundValue> {
    let mut out = HashMap::new();
    let mut stack: Vec<&Node> = form.root.children.iter().collect();
    while let Some(node) = stack.pop() {
        let (som, value) = match node {
            Node::Subform(s) => {
                stack.extend(s.children.iter());
                continue;
            }
            Node::Field(f) if f.data.is_some() => {
                (&f.som, BoundValue::Field(f.value.clone(), f.checked))
            }
            Node::ExclGroup(g) if g.data.is_some() => {
                let buttons = g.fields.iter().map(|f| (f.value.clone(), f.checked));
                (
                    &g.som,
                    BoundValue::Group(g.value.clone(), buttons.collect()),
                )
            }
            _ => continue,
        };
        if only.is_none_or(|keep| keep.contains_key(som)) {
            out.insert(som.clone(), value);
        }
    }
    out
}

/// Puts back the values `bound_values` saved.
fn restore_bound_values(form: &mut Form, saved: &HashMap<String, BoundValue>) {
    let mut stack: Vec<&mut Node> = form.root.children.iter_mut().collect();
    while let Some(node) = stack.pop() {
        match node {
            Node::Subform(s) => stack.extend(s.children.iter_mut()),
            Node::Field(f) => {
                if let Some(BoundValue::Field(value, checked)) = saved.get(&f.som) {
                    f.value = value.clone();
                    f.checked = *checked;
                }
            }
            Node::ExclGroup(g) => {
                if let Some(BoundValue::Group(value, buttons)) = saved.get(&g.som) {
                    g.value = value.clone();
                    for (f, (value, checked)) in g.fields.iter_mut().zip(buttons) {
                        f.value = value.clone();
                        f.checked = *checked;
                    }
                }
            }
            Node::Draw(_) => {}
        }
    }
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
/// not fillable or are floating fields (embedded in rich text), and of
/// fields that grow with their text (`grows_with_text`). Other fillable
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
                if !f.fillable() || !f.common.id.is_empty() || grows_with_text(f) {
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

/// Whether a field's height follows its text: a multi-line text field
/// without a fixed `h` whose `maxH` leaves room above its `minH` (a status
/// certificate's "give brief particulars" box). A fill of one lays the
/// form out again, as Acrobat grows the box and moves what follows.
fn grows_with_text(f: &types::Field) -> bool {
    let c = &f.common;
    c.h.is_none()
        && matches!(
            f.ui,
            types::Ui::TextEdit {
                multi_line: true,
                ..
            }
        )
        && c.max_h
            .is_none_or(|max| c.min_h.is_none_or(|min| max > min))
}

/// `SIGNATURE_KEY` of `doc`'s catalog.
pub fn stored_signature(doc: &Document<'_>) -> Option<u64> {
    let catalog = doc.resolve_id(doc.root()).as_dict()?;
    doc.dict_get(catalog, SIGNATURE_KEY)
        .as_i64()
        .map(|bits| bits as u64)
}

/// The hash `DATA_HASH_KEY` holds: FNV-1a of the datasets packet's text
/// as `packets::read` reads it (a leading byte order mark dropped), a
/// fixed value when there is none.
pub fn data_hash(datasets: Option<&str>) -> u64 {
    let mut hash = Fnv::default();
    match datasets {
        Some(text) => {
            hash.bytes(b"d");
            hash.bytes(text.strip_prefix('\u{feff}').unwrap_or(text).as_bytes());
        }
        None => hash.bytes(b"-"),
    }
    hash.0
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
