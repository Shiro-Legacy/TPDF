// SPDX-License-Identifier: MIT
//! Filled values written back into the datasets packet, so Acrobat (which
//! lays the form out from the XFA packets) shows what was filled here.
//! Each terminal field of a materialized document carries `DATA_KEY`, the
//! `DataPath` of its data node; a fill replaces that node's text with the
//! field's value (a check button's on or off save value, a choice's
//! export value, a text field's text) and rewrites the datasets stream in
//! the same journaled step as the fill, so undo restores both.
//!
//! Rules:
//! - The packet is `<xfa:datasets>` (or an `<xdp:xdp>` holding one); the
//!   paths start under its `xfa:data` child, made when missing.
//! - A step `(name, index)` is the index-th child element with that local
//!   name; missing ones (and their missing earlier siblings) are appended.
//! - A target that holds child elements is a data group, never replaced:
//!   that value is skipped.
//! - Everything else is kept as the reader read it (comments and processing
//!   instructions are not kept by `xml`). An XML declaration is written only
//!   when the packet had one: an array /XFA concatenates its packets into
//!   one document, where a declaration in the middle is an error.
//! - Bounded: path length by `limits::MAX_XML_DEPTH`, indexes by
//!   `MAX_INDEX`, values by `MAX_VALUES`.

use super::types::DataPath;
use super::xml::{self, Element, XmlNode};
use super::{XfaError, limits};

/// The largest occurrence index a path may name (siblings appended to
/// reach it).
pub const MAX_INDEX: usize = 10_000;
/// Values one call writes at most.
pub const MAX_VALUES: usize = 100_000;

/// `datasets_xml` with each path's node text replaced by its value. A
/// path whose node does not exist is created under `xfa:data` along the
/// path (missing ancestors added, indexes beyond the existing siblings
/// appended). Returns the new XML.
pub fn update_datasets(
    datasets_xml: &str,
    values: &[(DataPath, String)],
) -> Result<String, XfaError> {
    let text = datasets_xml
        .strip_prefix('\u{feff}')
        .unwrap_or(datasets_xml);
    let mut root = xml::parse(text).map_err(|e| XfaError::Xml(format!("datasets: {e}")))?;
    update_element(&mut root, values)?;
    if text.trim_start().starts_with("<?xml") {
        Ok(xml::serialize(&root))
    } else {
        Ok(xml::serialize_fragment(&root))
    }
}

/// `update_datasets` on a parsed packet.
pub fn update_element(root: &mut Element, values: &[(DataPath, String)]) -> Result<(), XfaError> {
    if values.len() > MAX_VALUES {
        return Err(XfaError::Limit("datasets values"));
    }
    let datasets: &mut Element = if root.local() == "datasets" {
        root
    } else {
        match root.child_mut("datasets") {
            Some(d) => d,
            None => return Err(XfaError::Unsupported("no xfa:datasets element".into())),
        }
    };
    let data = match child_index(datasets, "data", 0) {
        Some(at) => at,
        None => {
            let name = match datasets.name.rfind(':') {
                Some(i) => format!("{}:data", &datasets.name[..i]),
                None => "data".to_string(),
            };
            datasets.children.push(XmlNode::Element(Element {
                name,
                ..Element::default()
            }));
            datasets.children.len() - 1
        }
    };
    let Some(XmlNode::Element(data)) = datasets.children.get_mut(data) else {
        return Err(XfaError::Unsupported("no xfa:data element".into()));
    };
    for (path, value) in values {
        if path.0.is_empty() {
            continue;
        }
        if path.0.len() > limits::MAX_XML_DEPTH {
            return Err(XfaError::Limit("data path length"));
        }
        if path.0.iter().any(|(_, index)| *index > MAX_INDEX) {
            return Err(XfaError::Limit("data path index"));
        }
        let mut node: &mut Element = data;
        for (name, index) in &path.0 {
            let at = ensure(node, name, *index);
            node = match node.children.get_mut(at) {
                Some(XmlNode::Element(e)) => e,
                _ => return Err(XfaError::Unsupported("data path".into())),
            };
        }
        if node.has_elements() {
            continue;
        }
        node.set_text(value);
    }
    Ok(())
}

/// The position among `parent.children` of the `index`-th element with
/// local name `name`.
fn child_index(parent: &Element, name: &str, index: usize) -> Option<usize> {
    parent
        .children
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c, XmlNode::Element(e) if e.local() == name))
        .nth(index)
        .map(|(at, _)| at)
}

/// `child_index`, appending elements named `name` until it exists.
fn ensure(parent: &mut Element, name: &str, index: usize) -> usize {
    if let Some(at) = child_index(parent, name, index) {
        return at;
    }
    let have = parent.children_named(name).count();
    for _ in have..=index {
        parent.children.push(XmlNode::Element(Element {
            name: name.to_string(),
            ..Element::default()
        }));
    }
    parent.children.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(name: &str, children: Vec<XmlNode>) -> Element {
        Element {
            name: name.into(),
            attrs: Vec::new(),
            children,
        }
    }
    fn text(t: &str) -> XmlNode {
        XmlNode::Text(t.into())
    }
    fn path(text: &str) -> DataPath {
        DataPath::parse(text).unwrap()
    }

    #[test]
    fn existing_nodes_are_replaced_and_missing_ones_made() {
        let mut root = el(
            "xfa:datasets",
            vec![XmlNode::Element(el(
                "xfa:data",
                vec![XmlNode::Element(el(
                    "form1",
                    vec![
                        XmlNode::Element(el("Name", vec![text("old")])),
                        XmlNode::Element(el("Row", vec![XmlNode::Element(el("A", vec![]))])),
                    ],
                ))],
            ))],
        );
        update_element(
            &mut root,
            &[
                (path("form1[0]/Name[0]"), "new".into()),
                (path("form1[0]/Row[2]/A[0]"), "x".into()),
                (path("form1[0]/Row[0]"), "a group".into()),
                (path("form1[0]/Sub[0]/Check[0]"), "1".into()),
            ],
        )
        .unwrap();
        let data = root.child("data").unwrap();
        let form = data.child("form1").unwrap();
        assert_eq!(form.child("Name").unwrap().text(), "new");
        let rows: Vec<&Element> = form.children_named("Row").collect();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].has_elements(), "a data group is never replaced");
        assert_eq!(rows[2].child("A").unwrap().text(), "x");
        assert_eq!(
            form.child("Sub").unwrap().child("Check").unwrap().text(),
            "1"
        );
    }

    #[test]
    fn a_missing_data_element_is_made_and_bounds_hold() {
        let mut root = el("xfa:datasets", vec![]);
        update_element(&mut root, &[(path("f[0]"), "v".into())]).unwrap();
        assert_eq!(root.child("data").unwrap().name, "xfa:data");
        assert_eq!(root.child("data").unwrap().child("f").unwrap().text(), "v");
        let huge = DataPath(vec![("a".into(), MAX_INDEX + 1)]);
        assert!(matches!(
            update_element(&mut root, &[(huge, "v".into())]),
            Err(XfaError::Limit(_))
        ));
        let deep = DataPath(vec![("a".into(), 0); limits::MAX_XML_DEPTH + 1]);
        assert!(matches!(
            update_element(&mut root, &[(deep, "v".into())]),
            Err(XfaError::Limit(_))
        ));
        let mut other = el("template", vec![]);
        assert!(update_element(&mut other, &[]).is_err());
    }

    #[test]
    fn round_trip_through_text() {
        let xml = "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\">\
            <xfa:data><form1><Name>old</Name><C>0</C></form1></xfa:data></xfa:datasets>";
        let out = update_datasets(
            xml,
            &[
                (path("form1[0]/Name[0]"), "a < b".into()),
                (path("form1[0]/C[0]"), "1".into()),
            ],
        )
        .unwrap();
        assert!(!out.starts_with("<?xml"), "{out}");
        assert!(out.contains("<Name>a &lt; b</Name>"), "{out}");
        assert!(out.contains("<C>1</C>"), "{out}");
        assert!(update_datasets("not xml <", &[]).is_err());
    }
}
