// SPDX-License-Identifier: MIT
//! Data binding: the template's subforms expanded into instances
//! (`occur`, and one per matching data group for repeating subforms),
//! every node's SOM name set, every field's data node found (`bind
//! match`: by name under the parent's data node, `global` anywhere,
//! `dataRef` by SOM expression, `none` unbound) and its value taken from
//! the data when the node exists, else from the template. A check button
//! is `checked` when its value equals its on item; an `exclGroup`'s value
//! is its checked button's on item.
//!
//! Rules it follows:
//! - The data context of a node is its nearest bound ancestor's data
//!   node; an unnamed subform, or one with `match="none"`, passes its
//!   parent's context through.
//! - `once` matching consumes data nodes: the k-th node of a name under a
//!   parent binds the k-th template node (or instance) of that name there.
//!   Subforms take data groups, fields and groups take data values.
//! - A node bound to data that does not exist still gets the path where
//!   the node would be (after the existing same-named siblings), so a fill
//!   can create it; only `none`, an unresolved `dataRef`, and nodes under
//!   those have no path.
//! - SOM steps are `name[i]`, `i` counting same-named nodes (any kind) of
//!   the same named parent; unnamed fields, groups and page areas are
//!   `#field[i]`, `#exclGroup[i]`, `#pageArea[i]`; page-area statics are
//!   under `root.#pageSet[0].page[i]` and bind after the body.
//! - Every node of the result counts against `limits::MAX_FORM_NODES`, and
//!   a repeating subform is refused before it is copied when its copies
//!   would pass it.

use std::collections::HashMap;

use super::XfaError;
use super::limits::MAX_FORM_NODES;
use super::types::{BindMatch, DataPath, ExclGroup, Field, Form, Node, Subform, Value, ValueKind};
use super::xml::Element;

/// Binds `form` to `datasets` (the `xfa:datasets` root element, when the
/// document has one). Without data every subform has `occur.initial`
/// instances (at least `min`) and every field its template value.
pub fn bind(form: &mut Form, datasets: Option<&Element>) -> Result<(), XfaError> {
    let datasets = datasets.filter(|d| d.local() == "datasets");
    form.datasets = datasets.map(|d| std::sync::Arc::new(d.clone()));
    let data_root = datasets.and_then(|d| d.child("data"));
    let mut binder = Binder {
        data_root,
        slots: HashMap::new(),
        globals: HashMap::new(),
        som_counts: HashMap::new(),
        nodes: 0,
    };
    let top = Ctx {
        node: data_root,
        path: Some(DataPath::default()),
    };
    let mut root = std::mem::take(&mut form.root);
    binder.count(1)?;
    root.som = binder.som_name("", &root.common.name, "");
    root.instance = 0;
    let (data, ctx) = binder.subform_slot(&root, &top);
    root.data = data;
    let children = std::mem::take(&mut root.children);
    root.children = binder.children(children, &ctx, &root.som)?;
    // Page-master content, after the body (so it does not consume the
    // body's data nodes).
    let page_set = join(&root.som, "#pageSet[0]");
    for area in &mut form.page_set.areas {
        let som = binder.som_name(&page_set, &area.name, "#pageArea");
        let statics = std::mem::take(&mut area.statics);
        area.statics = binder.children(statics, &ctx, &som)?;
    }
    form.root = root;
    record_ids(form);
    Ok(())
}

/// Rebuilds `form.ids` (the shown value of every field with an `id`) from
/// the tree: after binding, and again after scripts set values.
pub fn record_ids(form: &mut Form) {
    let mut ids = std::collections::BTreeMap::new();
    let mut stack: Vec<&Node> = form.root.children.iter().collect();
    for area in &form.page_set.areas {
        stack.extend(area.statics.iter());
    }
    while let Some(node) = stack.pop() {
        match node {
            Node::Subform(s) => stack.extend(s.children.iter()),
            Node::Field(f) => record_id(form, &mut ids, f),
            Node::ExclGroup(g) => g.fields.iter().for_each(|f| record_id(form, &mut ids, f)),
            Node::Draw(_) => {}
        }
    }
    form.ids = ids;
}

/// A field's shown value (through its display picture) under its `id`.
fn record_id(form: &Form, ids: &mut std::collections::BTreeMap<String, String>, f: &Field) {
    if !f.common.id.is_empty() {
        ids.entry(f.common.id.clone())
            .or_insert_with(|| super::picture::shown(form, f));
    }
}

/// A node's data context: the data node (when it exists) and its path
/// (`None`: nothing under it binds by name).
#[derive(Clone)]
struct Ctx<'d> {
    node: Option<&'d Element>,
    path: Option<DataPath>,
}

/// A data node a template node binds: the node when it exists, and its
/// path.
struct Slot<'d> {
    node: Option<&'d Element>,
    path: DataPath,
}

/// The data children of one name and kind under one data node, and how
/// many `once` matches took.
struct Candidates<'d> {
    /// (node, index among all same-named siblings)
    list: Vec<(&'d Element, usize)>,
    /// Same-named siblings of any kind.
    total: usize,
    used: usize,
}

struct Binder<'d> {
    data_root: Option<&'d Element>,
    slots: HashMap<String, Candidates<'d>>,
    globals: HashMap<(String, bool), Option<(&'d Element, DataPath)>>,
    som_counts: HashMap<String, usize>,
    nodes: usize,
}

fn join(parent: &str, step: &str) -> String {
    if parent.is_empty() {
        step.to_string()
    } else {
        format!("{parent}.{step}")
    }
}

/// A data group (binds subforms): marked so, or with child elements, or
/// empty. A data value (binds fields and groups): not marked a group, and
/// without child elements unless they are rich text.
fn is_kind(e: &Element, group: bool) -> bool {
    let marked = e.attr("dataNode");
    if group {
        match marked {
            Some("dataGroup") => true,
            Some("dataValue") => false,
            _ => e.has_elements() || e.text().trim().is_empty(),
        }
    } else {
        match marked {
            Some("dataValue") => true,
            Some("dataGroup") => false,
            _ => !e.has_elements() || e.attr("contentType").is_some() || e.child("body").is_some(),
        }
    }
}

impl<'d> Binder<'d> {
    fn count(&mut self, n: usize) -> Result<(), XfaError> {
        self.nodes = self.nodes.saturating_add(n);
        if self.nodes > MAX_FORM_NODES {
            return Err(XfaError::Limit("form nodes after binding"));
        }
        Ok(())
    }

    /// The SOM name of a node named `name` under `parent`; an unnamed one
    /// is `unnamed[i]`, or `parent` itself when `unnamed` is empty.
    fn som_name(&mut self, parent: &str, name: &str, unnamed: &str) -> String {
        let step = if name.is_empty() { unnamed } else { name };
        if step.is_empty() {
            return parent.to_string();
        }
        let counter = self
            .som_counts
            .entry(format!("{parent}\u{0}{step}"))
            .or_insert(0);
        let index = *counter;
        *counter += 1;
        join(parent, &format!("{step}[{index}]"))
    }

    fn candidates(
        &mut self,
        ctx: &Ctx<'d>,
        name: &str,
        group: bool,
    ) -> Option<&mut Candidates<'d>> {
        let path = ctx.path.as_ref()?;
        let key = format!("{path}\u{0}{}\u{0}{name}", u8::from(group));
        let node = ctx.node;
        Some(self.slots.entry(key).or_insert_with(|| {
            let mut list = Vec::new();
            let mut total = 0usize;
            for child in node.into_iter().flat_map(Element::elements) {
                if child.local() != name {
                    continue;
                }
                if is_kind(child, group) {
                    list.push((child, total));
                }
                total += 1;
            }
            Candidates {
                list,
                total,
                used: 0,
            }
        }))
    }

    /// Data nodes of this name and kind not yet matched.
    fn remaining(&mut self, ctx: &Ctx<'d>, name: &str, group: bool) -> usize {
        self.candidates(ctx, name, group)
            .map_or(0, |c| c.list.len().saturating_sub(c.used))
    }

    /// The next `once` match, or where it would be.
    fn next_slot(&mut self, ctx: &Ctx<'d>, name: &str, group: bool) -> Option<Slot<'d>> {
        let base = ctx.path.clone()?;
        let c = self.candidates(ctx, name, group)?;
        let k = c.used;
        c.used += 1;
        let (node, index) = match c.list.get(k) {
            Some(&(e, i)) => (Some(e), i),
            None => (None, c.total + (k - c.list.len())),
        };
        let mut path = base;
        path.0.push((name.to_string(), index));
        Some(Slot { node, path })
    }

    /// The first data node of this name and kind anywhere under `xfa:data`,
    /// in document order.
    fn global(&mut self, name: &str, group: bool) -> Option<(&'d Element, DataPath)> {
        let key = (name.to_string(), group);
        if let Some(found) = self.globals.get(&key) {
            return found.clone();
        }
        let mut found = None;
        if let Some(root) = self.data_root {
            let mut stack: Vec<(&Element, DataPath)> = vec![(root, DataPath::default())];
            while let Some((e, path)) = stack.pop() {
                if !path.0.is_empty() && e.local() == name && is_kind(e, group) {
                    found = Some((e, path));
                    break;
                }
                // Children pushed last first: document order is a node, its
                // children, then its next sibling.
                let mut seen: HashMap<&str, usize> = HashMap::new();
                let at = stack.len();
                for child in e.elements() {
                    let index = seen.entry(child.local()).or_insert(0);
                    let mut child_path = path.clone();
                    child_path.0.push((child.local().to_string(), *index));
                    *index += 1;
                    stack.push((child, child_path));
                }
                stack[at..].reverse();
            }
        }
        self.globals.insert(key, found.clone());
        found
    }

    /// A `dataRef`: `$record.a.b`, `$data.a.b`, or `a.b[2]` relative to the
    /// context. Anything else (`..a`, other `$` roots, predicates) is not
    /// resolved.
    fn data_ref(&self, ctx: &Ctx<'d>, reference: &str) -> Option<(&'d Element, DataPath)> {
        let reference = reference.trim();
        let relative = !reference.starts_with('$');
        let (mut node, mut path, rest) = if let Some(rest) = reference.strip_prefix("$record") {
            let root = self.data_root?;
            let record = root.elements().next()?;
            (
                record,
                DataPath(vec![(record.local().to_string(), 0)]),
                rest,
            )
        } else if let Some(rest) = reference.strip_prefix("$data") {
            (self.data_root?, DataPath::default(), rest)
        } else if reference.starts_with('$') || reference.starts_with('.') || reference.is_empty() {
            return None;
        } else {
            (ctx.node?, ctx.path.clone()?, reference)
        };
        let rest = if relative {
            rest
        } else {
            match rest.strip_prefix('.') {
                Some(r) => r,
                None if rest.is_empty() => return Some((node, path)),
                None => return None,
            }
        };
        for step in rest.split('.') {
            let (name, index) = match step.find('[') {
                Some(open) => {
                    let inner = step[open + 1..].strip_suffix(']')?;
                    let index = if inner.trim() == "*" {
                        0
                    } else {
                        inner.trim().parse::<usize>().ok()?
                    };
                    (&step[..open], index)
                }
                None => (step, 0),
            };
            if name.is_empty() || name.starts_with('$') || name.starts_with('#') {
                return None;
            }
            node = node.elements().filter(|e| e.local() == name).nth(index)?;
            path.0.push((name.to_string(), index));
        }
        Some((node, path))
    }

    /// The data a node named `name` with this `bind` takes.
    fn slot(
        &mut self,
        ctx: &Ctx<'d>,
        name: &str,
        bind: &super::types::Bind,
        group: bool,
    ) -> Option<Slot<'d>> {
        match bind.matching {
            BindMatch::None => None,
            BindMatch::DataRef => self
                .data_ref(ctx, &bind.reference)
                .map(|(node, path)| Slot {
                    node: Some(node),
                    path,
                }),
            _ if name.is_empty() => None,
            BindMatch::Once => self.next_slot(ctx, name, group),
            BindMatch::Global => {
                let once = self.next_slot(ctx, name, group);
                if once.as_ref().is_some_and(|s| s.node.is_some()) {
                    return once;
                }
                match self.global(name, group) {
                    Some((node, path)) => Some(Slot {
                        node: Some(node),
                        path,
                    }),
                    None => once,
                }
            }
        }
    }

    /// A subform instance's data path and the context its children bind in.
    fn subform_slot(&mut self, s: &Subform, ctx: &Ctx<'d>) -> (Option<DataPath>, Ctx<'d>) {
        let passes_through = s.common.name.is_empty() && s.bind.matching != BindMatch::DataRef;
        if passes_through || s.bind.matching == BindMatch::None {
            return (None, ctx.clone());
        }
        match self.slot(ctx, &s.common.name, &s.bind, true) {
            Some(slot) => (
                Some(slot.path.clone()),
                Ctx {
                    node: slot.node,
                    path: Some(slot.path),
                },
            ),
            None => (
                None,
                Ctx {
                    node: None,
                    path: None,
                },
            ),
        }
    }

    fn children(
        &mut self,
        children: Vec<Node>,
        ctx: &Ctx<'d>,
        som: &str,
    ) -> Result<Vec<Node>, XfaError> {
        let mut out = Vec::with_capacity(children.len());
        for child in children {
            match child {
                Node::Subform(s) => self.expand(s, ctx, som, &mut out)?,
                Node::Field(f) => {
                    self.count(1)?;
                    out.push(Node::Field(self.field(f, ctx, som)));
                }
                Node::Draw(d) => {
                    self.count(1)?;
                    if !d.common.name.is_empty() {
                        self.som_name(som, &d.common.name, "");
                    }
                    out.push(Node::Draw(d));
                }
                Node::ExclGroup(g) => {
                    self.count(1 + g.fields.len())?;
                    out.push(Node::ExclGroup(self.group(g, ctx, som)));
                }
            }
        }
        Ok(out)
    }

    /// A template subform as its instances.
    fn expand(
        &mut self,
        mut s: Subform,
        ctx: &Ctx<'d>,
        som: &str,
        out: &mut Vec<Node>,
    ) -> Result<(), XfaError> {
        let occur = s.occur;
        let available = match s.bind.matching {
            BindMatch::Once | BindMatch::Global if !s.common.name.is_empty() => {
                self.remaining(ctx, &s.common.name, true)
            }
            _ => 0,
        };
        let mut instances = if available > 0 {
            available.max(occur.min)
        } else {
            occur.initial.max(occur.min)
        };
        if let Some(max) = occur.max {
            instances = instances.min(max);
        }
        if instances == 0 {
            return Ok(());
        }
        let size = node_count(&s);
        if self.nodes.saturating_add(size.saturating_mul(instances)) > MAX_FORM_NODES {
            return Err(XfaError::Limit("form nodes after binding"));
        }
        for i in 0..instances {
            let mut instance = if i + 1 == instances {
                std::mem::take(&mut s)
            } else {
                s.clone()
            };
            self.count(1)?;
            instance.instance = i;
            instance.som = self.som_name(som, &instance.common.name, "");
            let (data, child_ctx) = self.subform_slot(&instance, ctx);
            instance.data = data;
            let children = std::mem::take(&mut instance.children);
            instance.children = self.children(children, &child_ctx, &instance.som)?;
            out.push(Node::Subform(instance));
        }
        Ok(())
    }

    fn field(&mut self, mut f: Field, ctx: &Ctx<'d>, som: &str) -> Field {
        f.som = self.som_name(som, &f.common.name, "#field");
        if let Some(slot) = self.slot(ctx, &f.common.name, &f.bind, false) {
            if let Some(node) = slot.node {
                set_value(&mut f, node);
            }
            f.data = Some(slot.path);
        }
        if f.is_check() {
            f.checked = f.value.plain() == f.check_values().0;
        }
        f
    }

    fn group(&mut self, mut g: ExclGroup, ctx: &Ctx<'d>, som: &str) -> ExclGroup {
        g.som = self.som_name(som, &g.common.name, "#exclGroup");
        let mut data_value = None;
        if let Some(slot) = self.slot(ctx, &g.common.name, &g.bind, false) {
            data_value = slot.node.map(|n| n.text().trim().to_string());
            g.data = Some(slot.path);
        }
        let value = match data_value {
            Some(v) => v,
            None => g
                .fields
                .iter()
                .map(|f| (f.value.plain(), f.check_values().0))
                .find(|(value, on)| !value.is_empty() && value == on)
                .map(|(_, on)| on)
                .unwrap_or_default(),
        };
        let group_som = g.som.clone();
        for f in &mut g.fields {
            f.som = self.som_name(&group_som, &f.common.name, "#field");
            f.group = Some(group_som.clone());
            let (on, off) = f.check_values();
            f.checked = !value.is_empty() && on == value;
            f.value = Value::Text(if f.checked { on } else { off });
        }
        g.value = value;
        g
    }
}

/// A field's value from its data node: rich text as markup, text as
/// written, anything else trimmed. Image fields keep the template's.
fn set_value(f: &mut Field, node: &Element) {
    let rich = node.child("body").is_some()
        || node.attr("contentType").is_some_and(|c| c.contains("html"));
    f.value = match f.value_kind {
        ValueKind::Image => return,
        _ if rich && node.has_elements() => Value::Html(node.inner_xml()),
        ValueKind::Text | ValueKind::Html => Value::Text(node.text()),
        _ => Value::Text(node.text().trim().to_string()),
    };
}

/// Nodes in a subform's tree, itself included.
fn node_count(s: &Subform) -> usize {
    let mut count = 0usize;
    let mut stack: Vec<&Subform> = vec![s];
    while let Some(s) = stack.pop() {
        count += 1;
        for child in &s.children {
            match child {
                Node::Subform(c) => stack.push(c),
                Node::ExclGroup(g) => count += 1 + g.fields.len(),
                _ => count += 1,
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::{template, xml};

    fn form(body: &str) -> Form {
        template::read(&format!(
            "<template><subform name=\"form1\" layout=\"tb\">{body}</subform></template>"
        ))
        .unwrap()
    }

    fn data(inner: &str) -> Element {
        xml::parse(&format!(
            "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data>{inner}</xfa:data></xfa:datasets>"
        ))
        .unwrap()
    }

    /// Every field in the tree, in document order.
    fn fields(form: &Form) -> Vec<&Field> {
        let mut out = Vec::new();
        let mut stack: Vec<&Node> = form.root.children.iter().rev().collect();
        while let Some(node) = stack.pop() {
            match node {
                Node::Subform(s) => stack.extend(s.children.iter().rev()),
                Node::Field(f) => out.push(f),
                Node::ExclGroup(g) => out.extend(g.fields.iter()),
                Node::Draw(_) => {}
            }
        }
        out
    }

    fn subforms<'f>(parent: &'f Subform, name: &str) -> Vec<&'f Subform> {
        parent
            .children
            .iter()
            .filter_map(|n| match n {
                Node::Subform(s) if s.common.name == name => Some(s),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn repeats_one_instance_per_data_group() {
        let mut f = form(
            "<subform name=\"Table\" layout=\"table\"><subform name=\"Row\" layout=\"row\"><occur max=\"-1\"/>\
             <field name=\"Name\"/></subform></subform>",
        );
        let d = data(
            "<form1><Table><Row><Name>a</Name></Row><Row><Name>b</Name></Row><Other/><Row><Name> c </Name></Row></Table></form1>",
        );
        bind(&mut f, Some(&d)).unwrap();
        let table = subforms(&f.root, "Table")[0];
        let rows = subforms(table, "Row");
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows.iter().map(|r| r.instance).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(rows[2].som, "form1[0].Table[0].Row[2]");
        assert_eq!(
            rows[1].data.as_ref().unwrap().to_string(),
            "form1[0]/Table[0]/Row[1]"
        );
        let names: Vec<String> = fields(&f).iter().map(|f| f.value.plain()).collect();
        // Text values keep their spaces.
        assert_eq!(names, ["a", "b", " c "]);
        assert_eq!(fields(&f)[2].som, "form1[0].Table[0].Row[2].Name[0]");
        assert_eq!(
            fields(&f)[2].data.as_ref().unwrap().to_string(),
            "form1[0]/Table[0]/Row[2]/Name[0]"
        );
    }

    #[test]
    fn instances_without_data_come_from_occur() {
        let body = "<subform name=\"R\"><occur min=\"2\" max=\"5\" initial=\"3\"/><field name=\"x\"><value><text>t</text></value></field></subform>\
                    <subform name=\"Z\"><occur min=\"0\" max=\"1\" initial=\"0\"/></subform>\
                    <subform name=\"M\"><occur min=\"1\" max=\"2\"/></subform>";
        let mut f = form(body);
        bind(&mut f, None).unwrap();
        assert_eq!(subforms(&f.root, "R").len(), 3);
        assert_eq!(subforms(&f.root, "Z").len(), 0);
        assert_eq!(subforms(&f.root, "M").len(), 1);
        assert!(
            fields(&f)
                .iter()
                .all(|x| x.value == Value::Text("t".into()))
        );
        // Data with fewer groups than min: min; more than max: max.
        let mut f = form(body);
        bind(&mut f, Some(&data("<form1><R/><M/><M/><M/></form1>"))).unwrap();
        assert_eq!(subforms(&f.root, "R").len(), 2);
        assert_eq!(subforms(&f.root, "M").len(), 2);
        // The hypothetical path of a missing node follows the existing ones.
        let r = subforms(&f.root, "R");
        assert_eq!(r[1].data.as_ref().unwrap().to_string(), "form1[0]/R[1]");
    }

    #[test]
    fn som_names_skip_unnamed_subforms() {
        let mut f = form(
            "<subform><field name=\"A\"/><draw name=\"A\"/></subform><subform><field name=\"A\"/><field/></subform>\
             <subform name=\"S\"><subform><field name=\"B\"/></subform></subform>\
             <exclGroup name=\"G\"><field name=\"on\"/><field name=\"on\"/></exclGroup><exclGroup/>",
        );
        bind(&mut f, None).unwrap();
        let soms: Vec<&str> = fields(&f).iter().map(|f| f.som.as_str()).collect();
        assert_eq!(
            soms,
            [
                "form1[0].A[0]",
                "form1[0].A[2]",
                "form1[0].#field[0]",
                "form1[0].S[0].B[0]",
                "form1[0].G[0].on[0]",
                "form1[0].G[0].on[1]",
            ]
        );
        assert_eq!(f.root.som, "form1[0]");
        let Node::ExclGroup(g) = &f.root.children[3] else {
            panic!()
        };
        assert_eq!(g.som, "form1[0].G[0]");
        assert_eq!(g.fields[0].group.as_deref(), Some("form1[0].G[0]"));
        let Node::ExclGroup(unnamed) = &f.root.children[4] else {
            panic!()
        };
        assert_eq!(unnamed.som, "form1[0].#exclGroup[0]");
    }

    #[test]
    fn match_once_global_data_ref_and_none() {
        let mut f = form(
            "<subform name=\"S\">\
               <field name=\"a\"/><field name=\"a\"/>\
               <field name=\"g\"><bind match=\"global\"/></field>\
               <field name=\"r\"><bind match=\"dataRef\" ref=\"$record.Elsewhere.v[1]\"/></field>\
               <field name=\"r2\"><bind match=\"dataRef\" ref=\"$data.form1.Elsewhere.v\"/></field>\
               <field name=\"r3\"><bind match=\"dataRef\" ref=\"inner.v\"/></field>\
               <field name=\"r4\"><bind match=\"dataRef\" ref=\"..v\"/></field>\
               <field name=\"n\"><bind match=\"none\"/><value><text>keep</text></value></field>\
               <field name=\"missing\"><value><text>default</text></value></field>\
             </subform>",
        );
        let d = data(
            "<form1><S><a>1</a><a>2</a><inner><v>in</v></inner><n>data</n></S>\
             <Elsewhere><v>x</v><v>y</v><g>global</g></Elsewhere></form1>",
        );
        bind(&mut f, Some(&d)).unwrap();
        let all = fields(&f);
        let value = |i: usize| all[i].value.plain();
        let path = |i: usize| all[i].data.as_ref().map(|p| p.to_string());
        assert_eq!((value(0), value(1)), ("1".into(), "2".into()));
        assert_eq!(path(1).as_deref(), Some("form1[0]/S[0]/a[1]"));
        assert_eq!(value(2), "global");
        assert_eq!(path(2).as_deref(), Some("form1[0]/Elsewhere[0]/g[0]"));
        assert_eq!(value(3), "y");
        assert_eq!(path(3).as_deref(), Some("form1[0]/Elsewhere[0]/v[1]"));
        assert_eq!(value(4), "x");
        assert_eq!(value(5), "in");
        assert_eq!((value(6), path(6)), (String::new(), None));
        assert_eq!((value(7), path(7)), ("keep".into(), None));
        assert_eq!(value(8), "default");
        assert_eq!(path(8).as_deref(), Some("form1[0]/S[0]/missing[0]"));
    }

    #[test]
    fn check_buttons_and_groups() {
        let check =
            "<ui><checkButton/></ui><items><integer>1</integer><integer>0</integer></items>";
        let mut f = form(&format!(
            "<field name=\"c1\">{check}</field><field name=\"c2\">{check}<value><integer>1</integer></value></field>\
             <exclGroup name=\"G\"><field name=\"x\"><ui><checkButton/></ui><items><text>A</text></items></field>\
             <field name=\"y\"><ui><checkButton/></ui><items><text>B</text></items></field></exclGroup>\
             <exclGroup name=\"H\"><field name=\"x\"><ui><checkButton/></ui><items><text>A</text></items></field>\
             <field name=\"y\"><ui><checkButton/></ui><items><text>B</text></items><value><text>B</text></value></field></exclGroup>"
        ));
        bind(
            &mut f,
            Some(&data("<form1><c1>1</c1><c2>0</c2><G> B </G></form1>")),
        )
        .unwrap();
        let all = fields(&f);
        assert!(all[0].checked && !all[1].checked);
        assert_eq!((all[2].checked, all[3].checked), (false, true));
        assert_eq!(all[3].value, Value::Text("B".into()));
        let Node::ExclGroup(g) = &f.root.children[2] else {
            panic!()
        };
        assert_eq!(g.value, "B");
        assert_eq!(g.data.as_ref().unwrap().to_string(), "form1[0]/G[0]");
        // No data: the template's checked button.
        let Node::ExclGroup(h) = &f.root.children[3] else {
            panic!()
        };
        assert_eq!(h.value, "B");
        assert!(!h.fields[0].checked && h.fields[1].checked);
    }

    #[test]
    fn ids_and_rich_values() {
        let mut f = form(
            "<field name=\"Type\" id=\"floating1\"><bind match=\"global\"/></field>\
             <field name=\"Rich\"><value><exData contentType=\"text/html\"/></value></field>\
             <field name=\"NoData\" id=\"f2\"><value><text>tmpl</text></value></field>",
        );
        let d = data(
            "<form1><Rich xfa:contentType=\"text/html\"><body xmlns=\"http://www.w3.org/1999/xhtml\"><p>r</p></body></Rich>\
             <Deep><Type>Common</Type></Deep></form1>",
        );
        bind(&mut f, Some(&d)).unwrap();
        assert_eq!(f.ids.get("floating1").map(String::as_str), Some("Common"));
        assert_eq!(f.ids.get("f2").map(String::as_str), Some("tmpl"));
        assert_eq!(
            fields(&f)[1].value,
            Value::Html("<body xmlns=\"http://www.w3.org/1999/xhtml\"><p>r</p></body>".into())
        );
    }

    #[test]
    fn page_statics_bind_after_the_body() {
        let mut f = template::read(
            "<template><subform name=\"form1\"><pageSet><pageArea name=\"P\"><field name=\"x\"/></pageArea></pageSet>\
             <field name=\"x\"/></subform></template>",
        )
        .unwrap();
        bind(&mut f, Some(&data("<form1><x>body</x><x>page</x></form1>"))).unwrap();
        let Node::Field(page) = &f.page_set.areas[0].statics[0] else {
            panic!()
        };
        assert_eq!(page.som, "form1[0].#pageSet[0].P[0].x[0]");
        assert_eq!(page.value.plain(), "page");
        assert_eq!(fields(&f)[0].value.plain(), "body");
    }

    #[test]
    fn odd_data_is_no_data() {
        let mut f = form("<field name=\"a\"><value><text>t</text></value></field>");
        let wrong_root = xml::parse("<data><form1><a>x</a></form1></data>").unwrap();
        bind(&mut f, Some(&wrong_root)).unwrap();
        assert_eq!(fields(&f)[0].value.plain(), "t");
        // Mixed content and a group where a value is wanted.
        let mut f = form("<field name=\"a\"/><field name=\"b\"/>");
        bind(
            &mut f,
            Some(&data(
                "<form1><a>x<i>y</i>z</a><b xfa:dataNode=\"dataGroup\">q</b></form1>",
            )),
        )
        .unwrap();
        let all = fields(&f);
        assert_eq!(all[0].value.plain(), "");
        assert_eq!(all[0].data.as_ref().unwrap().to_string(), "form1[0]/a[1]");
        assert_eq!(all[1].value, Value::Empty);
        assert_eq!(all[1].data.as_ref().unwrap().to_string(), "form1[0]/b[1]");
    }

    #[test]
    fn unbounded_occur_with_many_groups_is_bounded() {
        let mut f = form(
            "<subform name=\"R\"><occur max=\"-1\"/><field name=\"v\"/><draw name=\"d\"/></subform>",
        );
        let rows = "<R><v>1</v></R>".repeat(10_000);
        let d = data(&format!("<form1>{rows}</form1>"));
        let start = std::time::Instant::now();
        bind(&mut f, Some(&d)).unwrap();
        assert_eq!(subforms(&f.root, "R").len(), 10_000);
        assert!(start.elapsed().as_secs() < 5);
        // Past the node limit: refused before copying.
        let mut f = form(&format!(
            "<subform name=\"R\"><occur max=\"-1\"/>{}</subform>",
            "<field name=\"v\"/>".repeat(30)
        ));
        assert_eq!(
            bind(&mut f, Some(&d)),
            Err(XfaError::Limit("form nodes after binding"))
        );
        let mut f = form("<subform name=\"R\"><occur min=\"1000000\" max=\"-1\"/></subform>");
        assert!(matches!(bind(&mut f, None), Err(XfaError::Limit(_))));
    }

    /// The owner's bench form, when it is on this machine (never in CI).
    #[test]
    fn bench_bind() {
        let dir = "/Users/jiaqihou/Documents/tpdf-wt/scratch/xfa";
        let (Ok(template), Ok(datasets)) = (
            std::fs::read_to_string(format!("{dir}/template.xml")),
            std::fs::read_to_string(format!("{dir}/datasets.xml")),
        ) else {
            return;
        };
        let start = std::time::Instant::now();
        let mut f = template::read(&template).unwrap();
        let d = xml::parse(&datasets).unwrap();
        bind(&mut f, Some(&d)).unwrap();
        let elapsed = start.elapsed();
        println!("bench read+bind: {elapsed:?}");
        let all = fields(&f);
        println!("bench bound: {} fields, {} ids", all.len(), f.ids.len());
        let init = all
            .iter()
            .find(|x| x.som == "form1[0].StatusCertificate[0].Section3[0].Type[0].initFld[0]")
            .expect("initFld");
        assert_eq!(init.value.plain(), "1");
        assert_eq!(
            init.data.as_ref().unwrap().to_string(),
            "form1[0]/StatusCertificate[0]/Section3[0]/Type[0]/initFld[0]"
        );
        assert!(elapsed.as_millis() < 200, "read+bind took {elapsed:?}");
    }
}
