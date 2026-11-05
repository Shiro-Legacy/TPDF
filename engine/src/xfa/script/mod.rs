// SPDX-License-Identifier: MIT
//! The form's JavaScript, run where Acrobat would run it: `parse` reads a
//! script, `eval` runs it against the bound form. This module picks the
//! events: at materialize time every `initialize` event in document order
//! (a node before its children, a group before its buttons), then the
//! `ready` events (`$form` first, then `$layout`), as Acrobat fires them
//! at open; on a fill, the handlers of the filled node (`click` and
//! `change` for a check button or a radio group and its chosen button,
//! `change` and `exit` for anything else). FormCalc scripts are skipped;
//! so is every event once the runner's budget is spent.

pub mod eval;
pub mod parse;

pub use eval::{EventInfo, NodeRef, NodeView, Runner, Stopped, view};

use super::types::{DataPath, Event, Node};

/// What a run of events did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Scripts run to their end.
    pub ran: usize,
    /// FormCalc scripts, left alone.
    pub skipped: usize,
    /// Scripts a bound stopped part way (the runner's budget included).
    pub stopped: usize,
}

impl Report {
    fn add(&mut self, outcome: Result<(), Stopped>) {
        match outcome {
            Ok(()) => self.ran += 1,
            Err(Stopped) => self.stopped += 1,
        }
    }
}

impl Runner<'_> {
    /// The open-time events: `initialize` in document order, then `ready`.
    pub fn initialize(&mut self) -> Report {
        let mut report = Report::default();
        let nodes = self.document_order();
        self.run_activity(&nodes, "initialize", None, &mut report);
        self.run_activity(&nodes, "ready", Some("$form"), &mut report);
        self.run_activity(&nodes, "ready", Some("$layout"), &mut report);
        report
    }

    /// The handlers of the node bound to `path` after it was filled with
    /// `value` (the save value of a button, the text of anything else).
    pub fn fire(&mut self, path: &DataPath, value: &str) -> Report {
        let mut report = Report::default();
        let Some(target) = self.bound_node(path) else {
            return report;
        };
        let (button, chosen) = match view(self.form, &target) {
            Some(NodeView::Field(f)) => (f.is_check(), None),
            Some(NodeView::Group(g)) => {
                let chosen = g
                    .fields
                    .iter()
                    .position(|f| f.check_values().0 == value)
                    .map(|i| target.child(i));
                (true, chosen)
            }
            _ => return report,
        };
        let activities: &[&str] = if button {
            &["click", "change"]
        } else {
            &["change", "exit"]
        };
        for activity in activities {
            self.run_node(&target, activity, None, Some(value), &mut report);
        }
        if let Some(chosen) = chosen {
            for activity in activities {
                self.run_node(&chosen, activity, None, Some(value), &mut report);
            }
        }
        report
    }

    /// Every node in document order, a parent before its children.
    fn document_order(&self) -> Vec<NodeRef> {
        let mut ordered = Vec::new();
        let mut stack = vec![NodeRef::default()];
        while let Some(at) = stack.pop() {
            let Some(node) = view(self.form, &at) else {
                continue;
            };
            // Pushed in reverse so the first child is visited first.
            for i in (0..node.child_count()).rev() {
                stack.push(at.child(i));
            }
            ordered.push(at);
        }
        ordered
    }

    fn run_activity(
        &mut self,
        nodes: &[NodeRef],
        activity: &str,
        reference: Option<&str>,
        report: &mut Report,
    ) {
        for at in nodes {
            if self.exhausted() {
                return;
            }
            self.run_node(at, activity, reference, None, report);
        }
    }

    /// Runs every event of `at` with this activity (and `ref`, when given).
    fn run_node(
        &mut self,
        at: &NodeRef,
        activity: &str,
        reference: Option<&str>,
        new_text: Option<&str>,
        report: &mut Report,
    ) {
        let events: Vec<Event> = match view(self.form, at) {
            Some(NodeView::Subform(s)) => s.events.clone(),
            Some(NodeView::Field(f)) => f.events.clone(),
            Some(NodeView::Group(g)) => g.events.clone(),
            _ => return,
        };
        for event in events {
            if event.activity != activity
                || reference.is_some_and(|r| event.reference != r)
                || (reference.is_none() && activity == "ready")
            {
                continue;
            }
            if !event.javascript {
                report.skipped += 1;
                continue;
            }
            let info = EventInfo {
                activity: activity.to_string(),
                target: at.clone(),
                new_text: new_text.map(str::to_string),
            };
            report.add(self.run(&event.script, at, info));
        }
    }

    /// The field or group whose data path is `path`.
    fn bound_node(&self, path: &DataPath) -> Option<NodeRef> {
        fn walk(nodes: &[Node], at: &NodeRef, path: &DataPath) -> Option<NodeRef> {
            for (i, node) in nodes.iter().enumerate() {
                let here = at.child(i);
                match node {
                    Node::Subform(s) => {
                        if let Some(found) = walk(&s.children, &here, path) {
                            return Some(found);
                        }
                    }
                    Node::Field(f) if f.data.as_ref() == Some(path) => return Some(here),
                    Node::ExclGroup(g) if g.data.as_ref() == Some(path) => return Some(here),
                    _ => {}
                }
            }
            None
        }
        walk(&self.form.root.children, &NodeRef::default(), path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::types::{Form, Presence};
    use crate::xfa::{bind, template, xml};

    const TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb">
  <event activity="initialize"><script contentType="application/x-javascript">S.Log.rawValue = "root";</script></event>
  <subform name="S" layout="tb">
    <event activity="initialize"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",S";</script></event>
    <event activity="ready" ref="$layout"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",layout";</script></event>
    <event activity="ready" ref="$form"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",form";</script></event>
    <field name="Log"><ui><textEdit/></ui>
      <event activity="initialize"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",Log";</script></event>
    </field>
    <field name="Calc"><ui><textEdit/></ui>
      <event activity="initialize"><script contentType="application/x-formcalc">$ = 1</script></event>
    </field>
    <exclGroup name="G">
      <event activity="change"><script contentType="application/x-javascript">if (this.rawValue == "b") { Hidden.presence = "visible"; Shown.presence = "hidden"; } Log.rawValue = Log.rawValue + ",Gchange:" + xfa.event.newText;</script></event>
      <field name="A"><ui><checkButton/></ui><items><text>a</text><text>off</text></items></field>
      <field name="B"><ui><checkButton/></ui><items><text>b</text><text>off</text></items>
        <event activity="click"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",Bclick";</script></event>
      </field>
    </exclGroup>
    <field name="C"><ui><checkButton/></ui><items><integer>1</integer><integer>0</integer></items>
      <event activity="click"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",Cclick:" + this.rawValue;</script></event>
    </field>
    <field name="T"><ui><textEdit/></ui>
      <event activity="exit"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",Texit:" + xfa.event.target.rawValue;</script></event>
    </field>
    <subform name="Hidden" presence="hidden"><draw name="D"/></subform>
    <subform name="Shown"><draw name="D"/></subform>
  </subform>
</subform>
</template>"#;

    fn form(datasets: &str) -> Form {
        let mut form = template::read(TEMPLATE).unwrap();
        let datasets = xml::parse(datasets).unwrap();
        bind::bind(&mut form, Some(&datasets)).unwrap();
        form
    }

    fn log(form: &Form) -> String {
        let crate::xfa::types::Node::Subform(s) = &form.root.children[0] else {
            panic!();
        };
        let crate::xfa::types::Node::Field(f) = &s.children[0] else {
            panic!();
        };
        f.value.plain()
    }

    fn presence_of(form: &Form, name: &str) -> Presence {
        let crate::xfa::types::Node::Subform(s) = &form.root.children[0] else {
            panic!();
        };
        s.children
            .iter()
            .find(|n| n.common().name == name)
            .unwrap()
            .common()
            .presence
    }

    const DATA: &str = "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data>\
        <form1><S><Log>L</Log><G>b</G><C>1</C><T>hello</T></S></form1></xfa:data></xfa:datasets>";

    #[test]
    fn initialize_runs_in_document_order_then_ready() {
        let mut form = form(DATA);
        let mut runner = Runner::new(&mut form);
        let report = runner.initialize();
        assert_eq!(
            report,
            Report {
                ran: 5,
                skipped: 1,
                stopped: 0
            }
        );
        drop(runner);
        assert_eq!(log(&form), "root,S,Log,form,layout");
        // Data alone does not fire change handlers at open.
        assert_eq!(presence_of(&form, "Hidden"), Presence::Hidden);
    }

    #[test]
    fn a_fill_fires_the_node_and_its_chosen_button() {
        let mut form = form(DATA);
        let mut runner = Runner::new(&mut form);
        let group = DataPath::parse("form1[0]/S[0]/G[0]").unwrap();
        assert_eq!(runner.fire(&group, "b").ran, 2);
        let check = DataPath::parse("form1[0]/S[0]/C[0]").unwrap();
        assert_eq!(runner.fire(&check, "1").ran, 1);
        let text = DataPath::parse("form1[0]/S[0]/T[0]").unwrap();
        assert_eq!(runner.fire(&text, "hello").ran, 1);
        let missing = DataPath::parse("form1[0]/S[0]/Nope[0]").unwrap();
        assert_eq!(runner.fire(&missing, "x"), Report::default());
        drop(runner);
        assert_eq!(log(&form), "L,Gchange:b,Bclick,Cclick:1,Texit:hello");
        assert_eq!(presence_of(&form, "Hidden"), Presence::Visible);
        assert_eq!(presence_of(&form, "Shown"), Presence::Hidden);
    }
}
