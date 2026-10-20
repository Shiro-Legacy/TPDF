//! A virtual DOM: the parts of the Electron renderer's page that its logic
//! and its drive STATE read back (an element's text, value, hidden, disabled,
//! open or pressed state), keyed by the element id `index.html` gives it.
//!
//! The port keeps the renderer's logic as it was; where that logic wrote an
//! element and later read it (or STATE reports it), it writes here instead.
//! Pure presentation (classes, styles, layout) is not modelled: the Leptos UI
//! draws from the core's state, not from this map. The UI may still read a
//! few entries (status lines, notice bar) where they are the plain words.

use std::collections::BTreeMap;

/// One element's observable state. Absent = the element's initial state in
/// `index.html` (empty text and value, visible, enabled, closed, unpressed).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Node {
    pub text: String,
    pub value: String,
    pub hidden: bool,
    pub disabled: bool,
    /// `<dialog>.open`, `<details>.open`.
    pub open: bool,
    /// `aria-pressed` / `aria-checked` / `aria-selected` / a checkbox's `checked`.
    pub checked: bool,
    /// Any other attribute the logic reads back (`data-*`, `aria-*`, `title`).
    pub attrs: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct Dom {
    nodes: BTreeMap<String, Node>,
}

impl Dom {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declares an element's initial state (as `index.html` writes it).
    pub fn init(&mut self, id: &str, node: Node) {
        self.nodes.insert(id.to_string(), node);
    }

    pub fn node(&self, id: &str) -> Node {
        self.nodes.get(id).cloned().unwrap_or_default()
    }

    pub fn node_mut(&mut self, id: &str) -> &mut Node {
        self.nodes.entry(id.to_string()).or_default()
    }

    pub fn text(&self, id: &str) -> String {
        self.nodes
            .get(id)
            .map(|n| n.text.clone())
            .unwrap_or_default()
    }
    pub fn set_text(&mut self, id: &str, text: impl Into<String>) {
        self.node_mut(id).text = text.into();
    }

    pub fn value(&self, id: &str) -> String {
        self.nodes
            .get(id)
            .map(|n| n.value.clone())
            .unwrap_or_default()
    }
    pub fn set_value(&mut self, id: &str, value: impl Into<String>) {
        self.node_mut(id).value = value.into();
    }

    pub fn hidden(&self, id: &str) -> bool {
        self.nodes.get(id).is_some_and(|n| n.hidden)
    }
    pub fn set_hidden(&mut self, id: &str, hidden: bool) {
        self.node_mut(id).hidden = hidden;
    }

    pub fn disabled(&self, id: &str) -> bool {
        self.nodes.get(id).is_some_and(|n| n.disabled)
    }
    pub fn set_disabled(&mut self, id: &str, disabled: bool) {
        self.node_mut(id).disabled = disabled;
    }

    pub fn is_open(&self, id: &str) -> bool {
        self.nodes.get(id).is_some_and(|n| n.open)
    }
    pub fn set_open(&mut self, id: &str, open: bool) {
        self.node_mut(id).open = open;
    }

    pub fn checked(&self, id: &str) -> bool {
        self.nodes.get(id).is_some_and(|n| n.checked)
    }
    pub fn set_checked(&mut self, id: &str, checked: bool) {
        self.node_mut(id).checked = checked;
    }

    pub fn attr(&self, id: &str, name: &str) -> Option<String> {
        self.nodes.get(id)?.attrs.get(name).cloned()
    }
    pub fn set_attr(&mut self, id: &str, name: &str, value: impl Into<String>) {
        self.node_mut(id)
            .attrs
            .insert(name.to_string(), value.into());
    }
    pub fn remove_attr(&mut self, id: &str, name: &str) {
        self.node_mut(id).attrs.remove(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_writes() {
        let mut dom = Dom::new();
        assert_eq!(dom.text("status"), "");
        assert!(!dom.hidden("error"));
        dom.set_text("status", "Saved");
        dom.set_hidden("error", true);
        dom.set_attr("tab", "aria-selected", "true");
        assert_eq!(dom.text("status"), "Saved");
        assert!(dom.hidden("error"));
        assert_eq!(dom.attr("tab", "aria-selected").as_deref(), Some("true"));
    }
}
