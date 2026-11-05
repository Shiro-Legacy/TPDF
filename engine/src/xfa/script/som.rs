// SPDX-License-Identifier: MIT
//! SOM references that name several nodes, and the data model. Both
//! script languages reach the form through here when a reference can
//! give more than one node (`Row[*]`, `..Amount`, `.*`,
//! `xfa.resolveNodes`) or names data (`$record`, `$data`, `!name`).
//!
//! Rules:
//! - A step keeps one node (`name`, `name[n]`: the n-th of the name,
//!   unnamed subforms transparent as for every name), all of them
//!   (`name[*]`), or the one a relative index gives (`name[-1]`,
//!   `name[+1]`: counted from the index of the node of that name the
//!   script runs in or under, 0 when there is none). `#class[n]` counts
//!   in the scope of the nearest named node at or above the cursor: its
//!   children in document order, each unnamed subform followed by its
//!   own; `*` keeps every child; `parent` the parent.
//! - `A..name` keeps the nodes named `name` anywhere under `A`, in
//!   document order (the index picks among all of them).
//! - The first step of an expression searches the scope chain from the
//!   script's node, as a name does; `$`, `$form`, `xfa.form`, `$record`
//!   (the data root's first group), `$data` (the data root),
//!   `xfa.datasets`, `xfa.record` and `!name` (a datasets child) start
//!   elsewhere.
//! - Data nodes are read through the form: a data value bound to a field
//!   reads the field's current value (so what scripts set shows); any
//!   other the packet's text as bound. Data nodes are not written.
//! - Bounded: a list holds at most `MAX_LIST` items; the callers charge
//!   one step per item to the script budgets.

use std::collections::HashMap;
use std::rc::Rc;

use super::eval::{NodeRef, NodeView, Runner, Value, view};
use crate::xfa::limits::MAX_FORM_NODES;
use crate::xfa::types::DataPath;
use crate::xfa::xml::Element;

/// Items one reference gives, at most.
pub const MAX_LIST: usize = MAX_FORM_NODES;
/// Levels a descendant search goes down, at most.
const MAX_DESCENT: usize = 512;

/// A data node: its path from the `datasets` element, each step a local
/// name and its index among same-named siblings.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct DataRef(pub Rc<[(Rc<str>, usize)]>);

impl DataRef {
    fn child(&self, name: &str, index: usize) -> DataRef {
        let mut steps: Vec<(Rc<str>, usize)> = self.0.to_vec();
        steps.push((Rc::from(name), index));
        DataRef(steps.into())
    }
    fn parent(&self) -> Option<DataRef> {
        let (_, rest) = self.0.split_last()?;
        Some(DataRef(rest.into()))
    }
}

/// Which of the nodes a step names it keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    At(usize),
    All,
    Rel(i64),
}

/// What a step selects.
#[derive(Clone, Copy, Debug)]
pub enum Sel<'a> {
    Name(&'a str),
    Class(&'a str),
    /// `*`: every child.
    Any,
    Parent,
}

impl<'a> Sel<'a> {
    /// A step's text as a selection: `#class`, `*`, `parent` or a name.
    pub fn of(text: &'a str) -> Sel<'a> {
        match text {
            "*" => Sel::Any,
            "parent" => Sel::Parent,
            t if t.starts_with('#') => Sel::Class(&t[1..]),
            t => Sel::Name(t),
        }
    }
}

/// The values of `list` a pick keeps (`base` is a relative pick's
/// origin).
fn pick<T: Clone>(mut list: Vec<T>, how: Pick, base: usize) -> Vec<T> {
    let index = match how {
        Pick::All => {
            list.truncate(MAX_LIST);
            return list;
        }
        Pick::At(n) => Some(n),
        Pick::Rel(d) => usize::try_from(base as i64 + d).ok(),
    };
    index
        .and_then(|i| list.get(i).cloned())
        .into_iter()
        .collect()
}

/// A list's values as one value: all of them for `[*]` (a list), else
/// the one kept (`Undefined` when none).
pub fn collapse(values: Vec<Value>, how: Pick) -> Value {
    match how {
        Pick::All => Value::List(values.into()),
        _ => values.into_iter().next().unwrap_or(Value::Undefined),
    }
}

impl Runner<'_> {
    /// One node visited, charged at the next `Runner::charge`.
    fn visit(&self) {
        self.visits.set(self.visits.get().saturating_add(1));
    }

    // ------------------------------------------------------------ form nodes

    /// The children of `at` named `name`, unnamed subforms searched
    /// through, in document order.
    pub(super) fn all_named(&self, at: &NodeRef, name: &str) -> Vec<NodeRef> {
        let mut out = Vec::new();
        self.collect_named(at, name, &mut out, 0);
        out
    }

    fn collect_named(&self, at: &NodeRef, name: &str, out: &mut Vec<NodeRef>, depth: usize) {
        let Some(parent) = view(self.form, at) else {
            return;
        };
        if depth > 32 {
            return;
        }
        for i in 0..parent.child_count() {
            if out.len() >= MAX_LIST {
                return;
            }
            self.visit();
            let child_ref = at.child(i);
            let Some(child) = view(self.form, &child_ref) else {
                continue;
            };
            if child.name() == name {
                out.push(child_ref);
            } else if let NodeView::Subform(s) = child {
                if s.common.name.is_empty() {
                    self.collect_named(&child_ref, name, out, depth + 1);
                }
            }
        }
    }

    /// Every node of `class` (`subform`, `field`, `draw`, `exclGroup`;
    /// `area` reads as a subform) in the scope of `at`: the nearest named
    /// node at or above `at`, its children in document order, each
    /// unnamed subform followed by its own.
    pub(super) fn all_class(&self, at: &NodeRef, class: &str) -> Vec<NodeRef> {
        let class = if class == "area" { "subform" } else { class };
        let mut scope = at.clone();
        while let Some(NodeView::Subform(s)) = view(self.form, &scope) {
            if !s.common.name.is_empty() {
                break;
            }
            match scope.parent() {
                Some(parent) => scope = parent,
                None => break,
            }
        }
        let mut out = Vec::new();
        self.collect_class(&scope, class, &mut out, 0);
        out
    }

    fn collect_class(&self, at: &NodeRef, class: &str, out: &mut Vec<NodeRef>, depth: usize) {
        let Some(parent) = view(self.form, at) else {
            return;
        };
        if depth > 32 {
            return;
        }
        for i in 0..parent.child_count() {
            if out.len() >= MAX_LIST {
                return;
            }
            self.visit();
            let child_ref = at.child(i);
            let Some(child) = view(self.form, &child_ref) else {
                continue;
            };
            if child.class() == class {
                out.push(child_ref.clone());
            }
            if let NodeView::Subform(s) = child {
                if s.common.name.is_empty() {
                    self.collect_class(&child_ref, class, out, depth + 1);
                }
            }
        }
    }

    fn children(&self, at: &NodeRef) -> Vec<NodeRef> {
        let count = view(self.form, at).map_or(0, |v| v.child_count());
        (0..count.min(MAX_LIST)).map(|i| at.child(i)).collect()
    }

    /// The nodes named `name` anywhere under `at`, in document order.
    fn descendants_named(&self, at: &NodeRef, name: &str) -> Vec<NodeRef> {
        let mut out = Vec::new();
        let mut stack = vec![(at.clone(), 0usize)];
        while let Some((node, depth)) = stack.pop() {
            self.visit();
            if depth >= MAX_DESCENT {
                continue;
            }
            let children = self.children(&node);
            for child in children.iter().rev() {
                stack.push((child.clone(), depth + 1));
            }
            if node != *at && view(self.form, &node).is_some_and(|v| v.name() == name) {
                out.push(node);
                if out.len() >= MAX_LIST {
                    break;
                }
            }
        }
        out
    }

    /// The index of the node named `name` that `this` is or sits in,
    /// among its same-named siblings; 0 when there is none.
    fn context_index(&self, this: &NodeRef, name: &str) -> usize {
        let mut at = Some(this.clone());
        while let Some(node) = at {
            if view(self.form, &node).is_some_and(|v| v.name() == name) {
                let mut scope = node.parent();
                // Unnamed subforms are transparent: count in the named one.
                while let Some(s) = scope.clone() {
                    match view(self.form, &s) {
                        Some(NodeView::Subform(sf)) if sf.common.name.is_empty() => {
                            scope = s.parent();
                        }
                        _ => break,
                    }
                }
                return scope
                    .map(|s| self.all_named(&s, name))
                    .and_then(|all| all.iter().position(|n| *n == node))
                    .unwrap_or(0);
            }
            at = node.parent();
        }
        0
    }

    // ------------------------------------------------------------------ data

    fn datasets(&self) -> Option<&Element> {
        self.form.datasets.as_deref()
    }

    /// The data node at `r`.
    pub(super) fn data_element(&self, r: &DataRef) -> Option<&Element> {
        let mut at = self.datasets()?;
        for (name, index) in r.0.iter() {
            at = (at.elements())
                .filter(|e| {
                    self.visit();
                    e.local() == &**name
                })
                .nth(*index)?;
        }
        Some(at)
    }

    /// `$data`: the `data` element.
    pub(super) fn data_root(&self) -> Option<DataRef> {
        let root = self.datasets()?.child("data")?;
        Some(DataRef::default().child(root.local(), 0))
    }

    /// `$record`: the data root's first element.
    pub(super) fn data_record(&self) -> Option<DataRef> {
        let data = self.data_root()?;
        let first = self.data_element(&data)?.elements().next()?;
        Some(data.child(first.local(), 0))
    }

    /// `!name`: the datasets children named `name`.
    pub(super) fn datasets_child(&self, name: &str) -> Vec<Value> {
        self.data_step(&DataRef::default(), Sel::Name(name), Pick::At(0), false)
    }

    fn data_children(&self, r: &DataRef, name: Option<&str>) -> Vec<DataRef> {
        let Some(e) = self.data_element(r) else {
            return Vec::new();
        };
        let mut seen: HashMap<&str, usize> = HashMap::new();
        let mut out = Vec::new();
        for child in e.elements() {
            self.visit();
            let index = seen.entry(child.local()).or_insert(0);
            if name.is_none_or(|n| n == child.local()) {
                out.push(r.child(child.local(), *index));
                if out.len() >= MAX_LIST {
                    break;
                }
            }
            *index += 1;
        }
        out
    }

    fn data_descendants(&self, r: &DataRef, name: &str) -> Vec<DataRef> {
        let mut out = Vec::new();
        let mut stack = vec![(r.clone(), 0usize)];
        while let Some((node, depth)) = stack.pop() {
            self.visit();
            if depth >= MAX_DESCENT {
                continue;
            }
            for child in self.data_children(&node, None).into_iter().rev() {
                stack.push((child, depth + 1));
            }
            if node != *r && node.0.last().is_some_and(|(n, _)| &**n == name) {
                out.push(node);
                if out.len() >= MAX_LIST {
                    break;
                }
            }
        }
        out
    }

    fn data_step(&self, r: &DataRef, sel: Sel, how: Pick, descendant: bool) -> Vec<Value> {
        let found = match sel {
            Sel::Name(name) if descendant => self.data_descendants(r, name),
            Sel::Name(name) => self.data_children(r, Some(name)),
            Sel::Any => {
                return self
                    .data_children(r, None)
                    .into_iter()
                    .map(Value::Data)
                    .collect();
            }
            Sel::Parent => r.parent().into_iter().collect(),
            Sel::Class(_) => Vec::new(),
        };
        let base = match (how, r.0.last()) {
            (Pick::Rel(_), Some((_, i))) => *i,
            _ => 0,
        };
        pick(found, how, base)
            .into_iter()
            .map(Value::Data)
            .collect()
    }

    /// A data node's value: the value of the field bound to it, when one
    /// is; else its text when it holds no elements. `Null` when empty.
    pub(super) fn data_value(&self, r: &DataRef) -> Value {
        if let Some(bound) = self.bound_to(r) {
            return self.raw_value(&bound);
        }
        match self.data_element(r) {
            Some(e) if !e.has_elements() => {
                let text = e.text();
                if text.is_empty() {
                    Value::Null
                } else {
                    Value::Str(Rc::from(text))
                }
            }
            _ => Value::Null,
        }
    }

    /// The field or group bound to data node `r`.
    fn bound_to(&self, r: &DataRef) -> Option<NodeRef> {
        let (first, rest) = r.0.split_first()?;
        if &*first.0 != "data" || first.1 != 0 {
            return None;
        }
        let path = DataPath(rest.iter().map(|(n, i)| (n.to_string(), *i)).collect());
        self.bound_map().get(&path).cloned()
    }

    /// Data paths to the fields and groups bound to them (the first in
    /// document order), built on first use.
    fn bound_map(&self) -> &HashMap<DataPath, NodeRef> {
        self.bound.get_or_init(|| {
            let mut map = HashMap::new();
            let mut stack = vec![NodeRef::default()];
            while let Some(at) = stack.pop() {
                match view(self.form, &at) {
                    Some(NodeView::Field(f)) => {
                        if let Some(p) = &f.data {
                            map.entry(p.clone()).or_insert(at);
                        }
                    }
                    Some(NodeView::Group(g)) => {
                        if let Some(p) = &g.data {
                            map.entry(p.clone()).or_insert(at.clone());
                        }
                    }
                    Some(NodeView::Subform(s)) => {
                        stack.extend((0..s.children.len()).rev().map(|i| at.child(i)));
                    }
                    _ => {}
                }
            }
            map
        })
    }

    /// `data.name` for a data node: `value`/`rawValue`, `name`, `isNull`,
    /// `parent`, or its first child of that name.
    pub(super) fn data_member(&self, r: &DataRef, name: &str) -> Value {
        match name {
            "value" | "rawValue" => self.data_value(r),
            "isNull" => Value::Bool(matches!(self.data_value(r), Value::Null)),
            "name" => {
                r.0.last()
                    .map_or(Value::Undefined, |(n, _)| Value::Str(n.clone()))
            }
            "parent" => r.parent().map_or(Value::Undefined, Value::Data),
            "index" => Value::Num(r.0.last().map_or(0.0, |(_, i)| *i as f64)),
            _ => self
                .data_step(r, Sel::Name(name), Pick::At(0), false)
                .into_iter()
                .next()
                .unwrap_or(Value::Undefined),
        }
    }

    // ----------------------------------------------------------------- steps

    /// One step from `item` (a form node or a data node): the values it
    /// selects. `this` is the script's node (for relative indexes).
    pub(super) fn step(
        &self,
        item: &Value,
        sel: Sel,
        how: Pick,
        descendant: bool,
        this: &NodeRef,
    ) -> Vec<Value> {
        let at = match item {
            Value::Node(at) => at,
            Value::Data(r) => return self.data_step(r, sel, how, descendant),
            _ => return Vec::new(),
        };
        let found = match sel {
            Sel::Name(name) if descendant => self.descendants_named(at, name),
            Sel::Name(name) => self.all_named(at, name),
            Sel::Class(class) => self.all_class(at, class),
            Sel::Any => self.children(at),
            Sel::Parent => at.parent().into_iter().collect(),
        };
        let base = match (how, sel) {
            (Pick::Rel(_), Sel::Name(name)) => self.context_index(this, name),
            _ => 0,
        };
        let how = match (sel, how) {
            (Sel::Any, Pick::At(0)) if !descendant => Pick::All,
            _ => how,
        };
        pick(found, how, base)
            .into_iter()
            .map(Value::Node)
            .collect()
    }

    /// The first step of a reference: `name` (or `#class`) searched in
    /// `base`'s scope chain, the nearest scope holding one.
    pub(super) fn scope_step(&self, base: &NodeRef, sel: Sel, how: Pick) -> Vec<Value> {
        let mut at = base.clone();
        loop {
            let found = self.step(&Value::Node(at.clone()), sel, how, false, base);
            if !found.is_empty() {
                return found;
            }
            match at.parent() {
                Some(parent) => at = parent,
                None => {
                    let root_named = matches!(sel, Sel::Name(n) if n == self.form.root.common.name);
                    return if root_named && matches!(how, Pick::At(0) | Pick::All) {
                        vec![Value::Node(at)]
                    } else {
                        Vec::new()
                    };
                }
            }
        }
    }

    /// Every value a SOM expression names, relative to `base` (see the
    /// module rules). Empty for anything not read (predicates, other
    /// models).
    pub(super) fn resolve_all(&self, base: &NodeRef, expression: &str) -> Vec<Value> {
        enum Cursor {
            Scope,
            Model,
            FormModel,
            Items(Vec<Value>),
        }
        let Some(steps) = split_steps(expression) else {
            return Vec::new();
        };
        let mut cursor = Cursor::Scope;
        for (text, descendant) in &steps {
            let Some((name, how)) = parse_step(text) else {
                return Vec::new();
            };
            cursor = match cursor {
                Cursor::Scope if *descendant => Cursor::Items(self.step(
                    &Value::Node(base.clone()),
                    Sel::of(name),
                    how,
                    true,
                    base,
                )),
                Cursor::Scope => match name {
                    "xfa" => Cursor::Model,
                    "$form" => Cursor::FormModel,
                    "$" => Cursor::Items(vec![Value::Node(base.clone())]),
                    "$record" => {
                        Cursor::Items(self.data_record().into_iter().map(Value::Data).collect())
                    }
                    "$data" => {
                        Cursor::Items(self.data_root().into_iter().map(Value::Data).collect())
                    }
                    n if n.starts_with('!') => Cursor::Items(self.datasets_child(&n[1..])),
                    n if n.starts_with('$') => return Vec::new(),
                    "*" => Cursor::Items(self.step(
                        &Value::Node(base.clone()),
                        Sel::Any,
                        how,
                        false,
                        base,
                    )),
                    n => Cursor::Items(self.scope_step(base, Sel::of(n), how)),
                },
                Cursor::Model => match name {
                    "form" => Cursor::FormModel,
                    "datasets" if self.datasets().is_some() => {
                        Cursor::Items(vec![Value::Data(DataRef::default())])
                    }
                    "record" => {
                        Cursor::Items(self.data_record().into_iter().map(Value::Data).collect())
                    }
                    _ => return Vec::new(),
                },
                Cursor::FormModel if *descendant => {
                    let root = Value::Node(NodeRef::default());
                    let mut found = self.step(&root, Sel::of(name), how, true, base);
                    let root_named = self.form.root.common.name == name;
                    if root_named && matches!(how, Pick::At(0) | Pick::All) {
                        found.insert(0, root);
                    }
                    Cursor::Items(found)
                }
                Cursor::FormModel => {
                    let root = self.form.root.common.name == name || name == "#subform";
                    if root && matches!(how, Pick::At(0) | Pick::All) {
                        Cursor::Items(vec![Value::Node(NodeRef::default())])
                    } else {
                        return Vec::new();
                    }
                }
                Cursor::Items(items) => {
                    let mut next = Vec::new();
                    for item in &items {
                        next.extend(self.step(item, Sel::of(name), how, *descendant, base));
                        if next.len() >= MAX_LIST {
                            next.truncate(MAX_LIST);
                            break;
                        }
                    }
                    Cursor::Items(next)
                }
            };
        }
        match cursor {
            Cursor::Items(items) => items,
            _ => Vec::new(),
        }
    }
}

/// The steps of a SOM expression (`.` outside brackets), each with
/// whether a `..` came before it. `None` when it does not split (an
/// empty step, an unbalanced bracket).
pub fn split_steps(expression: &str) -> Option<Vec<(String, bool)>> {
    let mut steps = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut descendant = false;
    let mut dots = 0usize;
    for c in expression.trim().chars() {
        if c == '.' && depth == 0 {
            if !current.is_empty() {
                steps.push((std::mem::take(&mut current), descendant));
            }
            dots += 1;
            continue;
        }
        if current.is_empty() {
            // A step begins: after one dot, or `..` (which searches down).
            descendant = match (dots, steps.is_empty()) {
                (0, true) | (1, false) => false,
                (2, _) => true,
                _ => return None,
            };
            dots = 0;
        }
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth = depth.checked_sub(1)?,
            _ => {}
        }
        current.push(c);
    }
    if current.is_empty() || depth != 0 || steps.len() > 256 {
        return None;
    }
    steps.push((current, descendant));
    Some(steps)
}

/// `name`, `name[n]`, `name[*]`, `name[+n]` or `name[-n]` as the name
/// and pick. `None` for a predicate or anything else.
pub fn parse_step(step: &str) -> Option<(&str, Pick)> {
    let Some(open) = step.find('[') else {
        return (!step.contains(['(', ')', ']'])).then_some((step, Pick::At(0)));
    };
    let inner = step[open + 1..].strip_suffix(']')?.trim();
    let how = if inner == "*" {
        Pick::All
    } else if let Some(n) = inner.strip_prefix('+') {
        Pick::Rel(n.trim().parse().ok()?)
    } else if let Some(n) = inner.strip_prefix('-') {
        Pick::Rel(-n.trim().parse::<i64>().ok()?)
    } else {
        Pick::At(inner.parse().ok()?)
    };
    Some((&step[..open], how))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_split_with_descendants_and_picks() {
        assert_eq!(
            split_steps("a.b[2].#subform[0]").unwrap(),
            [
                ("a".to_string(), false),
                ("b[2]".to_string(), false),
                ("#subform[0]".to_string(), false)
            ]
        );
        assert_eq!(
            split_steps("a..b[*].c").unwrap(),
            [
                ("a".to_string(), false),
                ("b[*]".to_string(), true),
                ("c".to_string(), false)
            ]
        );
        assert_eq!(split_steps("..b").unwrap(), [("b".to_string(), true)]);
        assert_eq!(split_steps("a...b"), None);
        assert_eq!(split_steps(".b"), None);
        assert_eq!(split_steps("a."), None);
        assert_eq!(split_steps("a[b.c].d").unwrap().len(), 2);
        assert_eq!(split_steps("a["), None);
        assert_eq!(split_steps(""), None);
        assert_eq!(parse_step("x[3]"), Some(("x", Pick::At(3))));
        assert_eq!(parse_step("x[*]"), Some(("x", Pick::All)));
        assert_eq!(parse_step("x[-1]"), Some(("x", Pick::Rel(-1))));
        assert_eq!(parse_step("x[+2]"), Some(("x", Pick::Rel(2))));
        assert_eq!(parse_step("x"), Some(("x", Pick::At(0))));
        assert_eq!(parse_step("x[a > 1]"), None);
        assert_eq!(pick(vec![1, 2, 3], Pick::Rel(-1), 1), [1]);
        assert_eq!(pick(vec![1, 2, 3], Pick::Rel(-1), 0), Vec::<i32>::new());
        assert_eq!(pick(vec![1, 2, 3], Pick::All, 0), [1, 2, 3]);
    }
}
