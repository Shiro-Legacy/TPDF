// SPDX-License-Identifier: MIT
//! Reopening a document this app materialized (`MARKER_KEY` in the
//! catalog). Such a file keeps the XFA packets and /NeedsRendering true,
//! so it can be filled elsewhere after we wrote it:
//!
//! - an XFA processor (Acrobat) lays it out from the packets and saves its
//!   fill into the datasets packet, leaving our widgets as they were;
//! - an AcroForm viewer (Preview, a browser) fills our widgets (/V, /AS)
//!   and leaves the datasets as they were.
//!
//! `needs_rebuild` tells which, if either, happened; `rebuild_at_open`
//! materializes the form again from its packets (the same pipeline as
//! `materialize`), after writing the widgets' values into the datasets in
//! the second case, so the scripts and the layout follow the fill.
//!
//! Rules:
//! - `DATA_HASH_KEY` (version 2) differs from `data_hash` of the datasets
//!   packet as read: `Reason::DatasetsChanged` (this app's fills keep it
//!   current). Otherwise the widgets are compared with the data.
//! - A terminal field carrying `DATA_KEY` has the value a fill writes into
//!   the datasets (`field_value`: /V as `field-set`'s sync writes it). For a
//!   check box or radio group the widgets' /AS is read too; when it and /V
//!   disagree, the one that differs from the data is taken (a viewer
//!   that updated only one of them).
//! - A value disagrees with the data when the text differs, the form bound
//!   to the datasets (no scripts) would not write that value either (a
//!   text field's /V is its value through its display picture), and the
//!   value read through the field's pictures (`picture::canonical`) is not
//!   the data's. The form is read only when some text differs.
//! - Disagreeing values with an unchanged hash: `Reason::WidgetsChanged`;
//!   without a stored hash (version 1): `Reason::DatasetsChanged` (the
//!   datasets are what Acrobat shows; ours could have been stale only
//!   through a fill elsewhere).
//! - Not decided (no rebuild): a document without the marker, without
//!   /NeedsRendering true or /XFA, whose packets do not read, and
//!   widgets changed in a one-stream /XFA or one without a datasets packet
//!   (whose datasets this app does not write).
//! - Bounded: the field walk by `MAX_WALK` nodes and `MAX_DEPTH`, the
//!   packets by `limits`; a rebuild is one materialization.

use std::collections::{HashMap, HashSet};

use crate::app::forms;
use crate::app::ops::fields::MAX_FIELD_OPTIONS;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

use super::emit::OFF_KEY;
use super::types::{DataPath, Form, Node};
use super::{
    DATA_HASH_KEY, DATA_KEY, MARKER_KEY, Materialized, Trigger, XfaError, bind, packets, picture,
    sync, xml,
};

/// Field tree nodes the walk visits at most.
pub const MAX_WALK: usize = 100_000;
/// Field tree depth the walk follows at most.
pub const MAX_DEPTH: usize = 64;

/// Why a reopened document is materialized again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The datasets packet is not the one this app wrote: it was filled
    /// by an XFA processor. The datasets win.
    DatasetsChanged,
    /// Widget values differ from the data they are bound to, the datasets
    /// unchanged: filled by an AcroForm viewer. The widgets win.
    WidgetsChanged,
}

impl Reason {
    /// The open info's `xfaRebuilt` value.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::DatasetsChanged => "datasets",
            Reason::WidgetsChanged => "widgets",
        }
    }
}

/// What `decide` found: the reason and, for `WidgetsChanged`, the data
/// values to write.
struct Decision {
    reason: Reason,
    updates: Vec<(DataPath, String)>,
    packets: packets::Packets,
}

/// Whether `doc`, a document this app materialized, was filled elsewhere
/// since and should be materialized again at open (module rules). `None`
/// for any other document.
pub fn needs_rebuild(doc: &Document<'_>) -> Option<Reason> {
    decide(doc).map(|d| d.reason)
}

/// `needs_rebuild` and, when it says so, the document materialized again
/// from its packets: for `WidgetsChanged` with the widgets' values written
/// into the datasets first. The caller opens `doc` as it is when the
/// rebuild fails.
pub fn rebuild_at_open(doc: &Document<'_>) -> Option<(Reason, Result<Vec<u8>, XfaError>)> {
    let decision = decide(doc)?;
    let reason = decision.reason;
    Some((reason, rebuild(doc, decision)))
}

fn rebuild(doc: &Document<'_>, decision: Decision) -> Result<Vec<u8>, XfaError> {
    let mut packets = decision.packets;
    let written = match decision.reason {
        Reason::DatasetsChanged => false,
        Reason::WidgetsChanged => {
            let text = packets.datasets.as_deref().ok_or(XfaError::NoXfa)?;
            packets.datasets = Some(sync::update_datasets(text, &decision.updates)?);
            true
        }
    };
    match super::build_from(doc, packets, written, Trigger::Open)? {
        Materialized::Written { bytes, .. } => Ok(bytes),
        Materialized::Unchanged { .. } => Err(XfaError::Write("nothing to write".into())),
    }
}

fn decide(doc: &Document<'_>) -> Option<Decision> {
    let catalog = doc.resolve_id(doc.root()).as_dict()?;
    if doc.dict_get(catalog, MARKER_KEY).is_null()
        || doc.dict_get(catalog, b"NeedsRendering").as_bool() != Some(true)
    {
        return None;
    }
    let packets = packets::read(doc).ok()?;
    let stored = doc.dict_get(catalog, DATA_HASH_KEY).as_i64();
    if stored.is_some_and(|bits| bits as u64 != super::data_hash(packets.datasets.as_deref())) {
        return Some(Decision {
            reason: Reason::DatasetsChanged,
            updates: Vec::new(),
            packets,
        });
    }
    let data = match &packets.datasets {
        Some(text) => Some(xml::parse(text).ok()?),
        None => None,
    };
    let lookup = |path: &DataPath| match &data {
        Some(root) => sync::lookup(root, path),
        None => Some(String::new()),
    };
    // Values whose text differs from the data's.
    let mut differ: Vec<(DataPath, String, String)> = Vec::new();
    for terminal in terminals(doc, catalog) {
        let Some(data) = lookup(&terminal.path) else {
            continue;
        };
        let value = match terminal.state {
            Some(state) if state != terminal.value && terminal.value == data => state,
            _ => terminal.value,
        };
        if value != data {
            differ.push((terminal.path, value, data));
        }
    }
    if differ.is_empty() {
        return None;
    }
    let mut form = super::read_form(&packets).ok()?;
    bind::bind(&mut form, data.as_ref()).ok()?;
    let expected = expected_values(&form);
    let mut updates = Vec::new();
    for (path, value, data) in differ {
        if expected.get(&path).is_some_and(|all| all.contains(&value)) {
            continue;
        }
        let canonical = super::first_field_at(&form, &path)
            .and_then(|f| picture::canonical(&form, f, &value))
            .unwrap_or(value);
        if canonical != data.trim() && canonical != data {
            updates.push((path, canonical));
        }
    }
    if updates.is_empty() {
        return None;
    }
    let reason = if stored.is_none() {
        Reason::DatasetsChanged
    } else {
        let writable = packets.whole.is_none()
            && packets.datasets.is_some()
            && packets.entries.iter().any(|(name, _)| name == b"datasets");
        if !writable {
            return None;
        }
        Reason::WidgetsChanged
    };
    Some(Decision {
        reason,
        updates,
        packets,
    })
}

/// A terminal field bound to data.
struct Terminal {
    path: DataPath,
    /// `field_value`.
    value: String,
    /// A button's value as its widgets' /AS show it.
    state: Option<String>,
}

/// The terminal fields carrying `DATA_KEY` under /AcroForm /Fields, in
/// tree order (bounded by `MAX_WALK` and `MAX_DEPTH`).
fn terminals(doc: &Document<'_>, catalog: &Dictionary) -> Vec<Terminal> {
    let mut out = Vec::new();
    let Some(acro) = doc.resolve(doc.dict_get(catalog, b"AcroForm")).as_dict() else {
        return out;
    };
    let Some(roots) = doc.resolve(doc.dict_get(acro, b"Fields")).as_array() else {
        return out;
    };
    let mut seen: HashSet<ObjectId> = HashSet::new();
    let mut stack: Vec<(&Object, usize)> = roots.iter().rev().map(|r| (r, 0)).collect();
    let mut visited = 0usize;
    while let Some((node, depth)) = stack.pop() {
        visited += 1;
        if visited > MAX_WALK || depth > MAX_DEPTH {
            break;
        }
        if let Some(id) = node.as_reference() {
            if !seen.insert(id) {
                continue;
            }
        }
        let Some(dict) = doc.resolve(node).as_dict() else {
            continue;
        };
        let path = (doc.resolve(doc.dict_get(dict, DATA_KEY)).as_string())
            .map(crate::object::decode_text_string)
            .and_then(|text| DataPath::parse(&text))
            .filter(|path| !path.0.is_empty());
        if let Some(path) = path {
            out.push(Terminal {
                path,
                value: field_value(doc, dict),
                state: button_state(doc, dict),
            });
            continue;
        }
        if let Some(kids) = doc.resolve(doc.dict_get(dict, b"Kids")).as_array() {
            stack.extend(kids.iter().rev().map(|k| (k, depth + 1)));
        }
    }
    out
}

/// The value a fill of the terminal field `head` writes into the
/// datasets (`field-set`'s sync): a text, a choice's export value (several
/// joined by newlines), a button's state name, or a check box's off value
/// (`OFF_KEY`) when it is off (an off radio group: empty).
pub fn field_value(doc: &Document<'_>, head: &Dictionary) -> String {
    match doc.resolve(doc.dict_get(head, b"V")) {
        Object::Name(name) if name.as_slice() == b"Off" => {
            forms::text_of(doc.resolve(doc.dict_get(head, OFF_KEY)))
        }
        Object::Name(name) => forms::name_text(name),
        Object::Array(items) => (items.iter())
            .take(MAX_FIELD_OPTIONS)
            .map(|item| forms::text_of(doc.resolve(item)))
            .collect::<Vec<_>>()
            .join("\n"),
        other => forms::text_of(other),
    }
}

/// A button field's value as its widgets' /AS show it: the first state
/// that is not /Off, else what `field_value` writes for off. `None` for
/// any other field, or one whose widgets carry no /AS.
fn button_state(doc: &Document<'_>, head: &Dictionary) -> Option<String> {
    if doc.resolve(doc.dict_get(head, b"FT")).as_name() != Some(b"Btn".as_slice()) {
        return None;
    }
    let mut widgets: Vec<&Dictionary> = Vec::new();
    if doc.resolve(doc.dict_get(head, b"Subtype")).as_name() == Some(b"Widget".as_slice()) {
        widgets.push(head);
    }
    if let Some(kids) = doc.resolve(doc.dict_get(head, b"Kids")).as_array() {
        widgets.extend(
            kids.iter()
                .take(MAX_WALK)
                .filter_map(|k| doc.resolve(k).as_dict()),
        );
    }
    let mut any = false;
    for widget in widgets {
        match doc.resolve(doc.dict_get(widget, b"AS")) {
            Object::Name(name) if name.as_slice() != b"Off" => {
                return Some(forms::name_text(name));
            }
            Object::Name(_) => any = true,
            _ => {}
        }
    }
    any.then(|| forms::text_of(doc.resolve(doc.dict_get(head, OFF_KEY))))
}

/// What the materializer writes as each bound field's `field_value`, for
/// `form` as bound (no scripts): by data path, every field's.
fn expected_values(form: &Form) -> HashMap<DataPath, Vec<String>> {
    let on = |f: &super::types::Field| {
        String::from_utf8_lossy(&super::emit::state_name(&f.check_values().0)).into_owned()
    };
    let mut out: HashMap<DataPath, Vec<String>> = HashMap::new();
    let mut stack: Vec<&Node> = form.root.children.iter().collect();
    while let Some(node) = stack.pop() {
        let (path, value) = match node {
            Node::Subform(s) => {
                stack.extend(s.children.iter());
                continue;
            }
            Node::Field(f) => {
                let Some(path) = &f.data else { continue };
                let value = if f.is_check() {
                    if f.checked { on(f) } else { f.check_values().1 }
                } else if matches!(f.ui, super::types::Ui::ChoiceList { .. }) {
                    f.value.plain()
                } else {
                    picture::shown(form, f)
                };
                (path, value)
            }
            Node::ExclGroup(g) => {
                let Some(path) = &g.data else { continue };
                let value = g.fields.iter().find(|f| f.checked).map(on);
                (path, value.unwrap_or_default())
            }
            Node::Draw(_) => continue,
        };
        out.entry(path.clone()).or_default().push(value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentLimits;

    const TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.3/"><subform name="form1" layout="tb"><pageSet><pageArea name="P"><contentArea x="0.25in" y="0.25in" w="8in" h="10.5in"/><medium stock="letter" short="8.5in" long="11in"/></pageArea></pageSet><subform name="S" layout="tb" w="8in"><field name="Name" w="3in" h="0.3in"><ui><textEdit/></ui></field><field name="Agree" w="0.3in" h="0.3in"><ui><checkButton/></ui><items><integer>1</integer><integer>0</integer></items></field></subform></subform></template>"#;
    const DATA: &str = r#"<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data><form1><S><Name>Ann</Name><Agree>0</Agree></S></form1></xfa:data></xfa:datasets>"#;

    fn hex(text: &str) -> String {
        text.bytes().map(|b| format!("{b:02x}")).collect::<String>() + ">"
    }

    /// A dynamic source with the template and `datasets` as an array /XFA.
    fn source(datasets: &str) -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 3 0 R /NeedsRendering true >>".to_string(),
            "<< /Type /Pages /Kids [4 0 R] /Count 1 >>".into(),
            "<< /Fields [] /XFA [(template) 5 0 R (datasets) 6 0 R] >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>".into(),
            format!(
                "<< /Length {} /Filter /ASCIIHexDecode >>\nstream\n{}\nendstream",
                TEMPLATE.len() * 2 + 1,
                hex(TEMPLATE)
            ),
            format!(
                "<< /Length {} /Filter /ASCIIHexDecode >>\nstream\n{}\nendstream",
                datasets.len() * 2 + 1,
                hex(datasets)
            ),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for at in offsets {
            out.extend(format!("{at:010} 00000 n \n").bytes());
        }
        out.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );
        out
    }

    fn materialized() -> Vec<u8> {
        let bytes = source(DATA);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        super::super::materialize(&doc).unwrap()
    }

    /// `bytes` with `edit` applied to the opened document, saved again.
    fn edited(bytes: &[u8], edit: impl FnOnce(&mut Document<'_>)) -> Vec<u8> {
        let mut doc = Document::open(bytes, DocumentLimits::default()).unwrap();
        edit(&mut doc);
        crate::writer::write_document(
            &doc,
            crate::writer::DocumentWriteLimits::default(),
            crate::writer::WriteOptions::APP,
        )
        .unwrap()
    }

    fn datasets_id(doc: &Document<'_>) -> ObjectId {
        let read = packets::read(doc).unwrap();
        read.entries
            .iter()
            .find(|(name, _)| name == b"datasets")
            .unwrap()
            .1
    }

    /// The terminal field whose `DATA_KEY` ends with `leaf`.
    fn field_id(doc: &Document<'_>, leaf: &str) -> ObjectId {
        doc.objects()
            .find(|o| {
                doc.resolve_id(o.id).as_dict().is_some_and(|d| {
                    d.get(DATA_KEY)
                        .and_then(Object::as_string)
                        .is_some_and(|s| s.ends_with(leaf.as_bytes()))
                })
            })
            .map(|o| o.id)
            .unwrap()
    }

    fn reason(bytes: &[u8]) -> Option<Reason> {
        let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
        needs_rebuild(&doc)
    }

    #[test]
    fn an_untouched_materialized_file_needs_nothing() {
        let bytes = materialized();
        assert_eq!(reason(&bytes), None);
        // Not ours: never decided.
        assert_eq!(reason(&source(DATA)), None);
    }

    #[test]
    fn a_replaced_datasets_stream_is_a_datasets_change() {
        let bytes = materialized();
        let changed = edited(&bytes, |doc| {
            let id = datasets_id(doc);
            doc.put_stream_decoded(
                id,
                Dictionary::new(),
                DATA.replace("Ann", "Zoe").into_bytes(),
            )
            .unwrap();
        });
        assert_eq!(reason(&changed), Some(Reason::DatasetsChanged));
        let doc = Document::open(&changed, DocumentLimits::default()).unwrap();
        let (_, rebuilt) = rebuild_at_open(&doc).unwrap();
        let rebuilt = rebuilt.unwrap();
        let again = Document::open(&rebuilt, DocumentLimits::default()).unwrap();
        let name = again
            .resolve_id(field_id(&again, "Name[0]"))
            .as_dict()
            .unwrap();
        assert_eq!(field_value(&again, name), "Zoe");
        assert_eq!(needs_rebuild(&again), None);
    }

    #[test]
    fn a_changed_widget_value_is_a_widgets_change() {
        let bytes = materialized();
        let changed = edited(&bytes, |doc| {
            let id = field_id(doc, "Name[0]");
            let mut dict = doc.resolve_id(id).as_dict().unwrap().clone();
            dict.insert(b"V".to_vec(), Object::text("Bea"));
            doc.put_value(id, Object::Dictionary(dict)).unwrap();
        });
        assert_eq!(reason(&changed), Some(Reason::WidgetsChanged));
        // A check box a viewer turned on through /AS only.
        let checked = edited(&bytes, |doc| {
            let id = field_id(doc, "Agree[0]");
            let mut dict = doc.resolve_id(id).as_dict().unwrap().clone();
            dict.insert(b"AS".to_vec(), Object::name("1"));
            doc.put_value(id, Object::Dictionary(dict)).unwrap();
        });
        assert_eq!(reason(&checked), Some(Reason::WidgetsChanged));
        let doc = Document::open(&checked, DocumentLimits::default()).unwrap();
        let rebuilt = rebuild_at_open(&doc).unwrap().1.unwrap();
        let again = Document::open(&rebuilt, DocumentLimits::default()).unwrap();
        let data = packets::read(&again).unwrap().datasets.unwrap();
        assert!(data.contains("<Agree>1</Agree>"), "{data}");
        assert_eq!(needs_rebuild(&again), None);
    }

    #[test]
    fn a_version_one_file_whose_datasets_disagree_is_a_datasets_change() {
        let bytes = materialized();
        let v1 = |data: &str| {
            let data = data.to_string();
            edited(&bytes, move |doc| {
                let root = doc.root();
                let mut catalog = doc.resolve_id(root).as_dict().unwrap().clone();
                catalog.remove(DATA_HASH_KEY);
                catalog.insert(MARKER_KEY.to_vec(), Object::Integer(1));
                doc.put_value(root, Object::Dictionary(catalog)).unwrap();
                let id = datasets_id(doc);
                doc.put_stream_decoded(id, Dictionary::new(), data.into_bytes())
                    .unwrap();
            })
        };
        assert_eq!(reason(&v1(DATA)), None);
        assert_eq!(
            reason(&v1(&DATA.replace("Ann", "Zoe"))),
            Some(Reason::DatasetsChanged)
        );
    }
}
